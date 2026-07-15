//! Take recorder: the capture stream written to a WAV, as evidence.
//!
//! Exists because the corpus in `testdata/` is the ground truth the pitch detector
//! is measured against, and a take only counts as ground truth if it is *the signal
//! the detector saw*. Recording outside the app made that a protocol held together
//! by hand (`testdata/README.md`: same source, don't touch the gain, no AGC, never
//! lossy). Recording from inside the capture path makes it a property of the
//! construction instead.
//!
//! Three invariants carry that, and each one is load-bearing:
//!
//! 1. **The tap is pre-gain.** It sits in the input callback, on the same raw
//!    samples the analysis rings get; `input_gain` is applied later and elsewhere,
//!    by the workers that drain those rings (`audio::core`). The recorder therefore
//!    *cannot* see the slider — not by agreement, but because the multiplication
//!    has not happened yet at the point the tap reads.
//!
//! 2. **Dropped samples are counted, and a take with any is disqualified.** The
//!    audio callback must never block, so a full ring costs a sample. For the
//!    analysis planes that is harmless (a stale frame; the next one is along in
//!    milliseconds). For a take it is a hole in the evidence that looks like
//!    nothing at all. See [`TakeReport::dropped`].
//!
//! 3. **The rate is the device's.** Not 44100. See [`TakeReport::sample_rate`].
//!
//! The app writes takes but still never *reads* an audio file: this is an output
//! path only. Replay into the engine would be a new input path, and that is a
//! separate decision (`memory/kickstart_recording_and_annotation.md`).

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
};
use std::thread;

use hound::{
    SampleFormat,
    WavSpec,
    WavWriter,
};
use ringbuf::HeapRb;
use ringbuf::traits::{
    Consumer,
    Observer,
    Producer,
    Split,
};

use super::{
    ANALYSIS_IDLE_SLEEP,
    AnalysisWorker,
    SampleConsumer,
    SampleProducer,
};
use crate::audio::types::{
    RecorderStatus,
    TakeReport,
};

/// Depth of the recorder's ring, in seconds of audio.
///
/// Deliberately far deeper than the analysis rings (0.5 s). Those can afford to
/// overflow — the cost is a stale frame. This one cannot: an overflow here is a
/// hole in a take. The depth buys the writer room to ride out a filesystem hiccup
/// without the callback ever having to drop, which is the difference between a
/// usable take and a wasted one.
const RECORDER_RING_SECONDS: u32 = 4;

/// How many samples the writer moves per wake-up. Matches the analysis workers'
/// drain quantum, for the same reason: bounded work per pass, no unbounded stall.
const RECORDER_BATCH: usize = 8192;

/// s16le, mono — the corpus format (`testdata/README.md`).
const RECORDER_BITS: u16 = 16;

pub(super) fn recorder_ring(sample_rate: u32) -> (SampleProducer, SampleConsumer) {
    HeapRb::<f32>::new((sample_rate * RECORDER_RING_SECONDS) as usize).split()
}

/// The recorder's end of the input fan-out.
///
/// A newtype rather than a bare producer because it owns the drop counter: the
/// rule "a sample the recorder never saw is a hole in the take" then lives in one
/// place instead of being restated at each capture path's callback. Both the cpal
/// and the Pulse path push through here.
pub(super) struct RecorderTap {
    prod:    SampleProducer,
    dropped: Arc<AtomicU64>,
}

impl RecorderTap {
    /// Realtime context: called from the input callback for every captured sample.
    /// Never blocks, never allocates — a full ring costs a counter bump, not a
    /// stall, and the counter is what stops that from being silent.
    pub(super) fn push(&mut self, sample: f32) {
        if self.prod.try_push(sample).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Control (UI → writer) and status (writer → UI) behind one lock.
///
/// A mutex is right here despite the audio thread being nearby: nothing in this
/// struct is touched per sample. The realtime side reaches the recorder only
/// through [`RecorderTap`], which is lock-free; this lock is taken by the writer
/// a few times a second and by the UI once a frame.
struct RecorderShared {
    /// `Some` while the user wants a take running, holding its destination.
    ///
    /// Edge-triggered: the writer compares this against the take it is holding
    /// open and acts on the difference, so a click is a state change rather than
    /// an event that can be missed.
    wanted: Option<PathBuf>,
    status: RecorderStatus,
}

/// Handle on the recorder, held by `AudioEngine` and cloned into `AudioContext`.
///
/// Lives above the capture (not inside it) so that a device switch rebuilding the
/// whole capture does not lose the take state — the new capture's writer picks up
/// the same shared intent.
#[derive(Clone)]
pub(super) struct RecorderHandle {
    shared:  Arc<Mutex<RecorderShared>>,
    /// Outside the mutex because [`RecorderTap::push`] bumps it from the realtime
    /// callback.
    dropped: Arc<AtomicU64>,
}

impl RecorderHandle {
    pub(super) fn new() -> Self {
        Self {
            shared:  Arc::new(Mutex::new(RecorderShared {
                wanted: None,
                status: RecorderStatus::Idle,
            })),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn tap(&self, prod: SampleProducer) -> RecorderTap {
        RecorderTap {
            prod,
            dropped: self.dropped.clone(),
        }
    }

    pub(super) fn start(&self, path: PathBuf) {
        self.shared.lock().unwrap().wanted = Some(path);
    }

    pub(super) fn stop(&self) {
        self.shared.lock().unwrap().wanted = None;
    }

    pub(super) fn status(&self) -> RecorderStatus {
        self.shared.lock().unwrap().status.clone()
    }

    fn publish(&self, status: RecorderStatus) {
        self.shared.lock().unwrap().status = status;
    }

    /// Report a take that died on us, and withdraw the request that produced it.
    /// Clearing `wanted` matters: without it the writer would see the intent still
    /// standing on the next pass and retry the same doomed path every few ms.
    fn fail(&self, message: String) {
        let mut shared = self.shared.lock().unwrap();
        shared.wanted = None;
        shared.status = RecorderStatus::Failed(message);
    }
}

/// A take the writer currently holds open.
struct OpenTake {
    path:            PathBuf,
    writer:          WavWriter<BufWriter<File>>,
    samples:         u64,
    /// The drop counter's value when this take opened. Drops that happened while
    /// idle belong to no take; only the delta across this one is a hole in it.
    dropped_at_open: u64,
}

impl OpenTake {
    fn open(
        path: PathBuf,
        sample_rate: u32,
        dropped: &AtomicU64,
        cons: &mut SampleConsumer,
    ) -> Result<Self, String> {
        // The corpus directory may not exist yet on a fresh checkout.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        }
        let spec = WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: RECORDER_BITS,
            sample_format: SampleFormat::Int,
        };
        let writer =
            WavWriter::create(&path, spec).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;

        // Drain the ring dry *after* the file exists and *before* the first written
        // sample, and snapshot the counter only then. Whatever is sitting in the
        // ring at this moment was captured before Record was pressed — including
        // everything that arrived during the file-creation syscalls above — and how
        // much of it there is depends on when the writer last woke. Without this,
        // takes would open with a nondeterministic slice of the past glued to the
        // front, and count drops that happened while nobody was recording.
        while cons.try_pop().is_some() {}

        Ok(Self {
            path,
            writer,
            samples: 0,
            dropped_at_open: dropped.load(Ordering::Relaxed),
        })
    }

    /// Extend the take. An error here ends it: a WAV with a failed write in the
    /// middle has a hole in it, which is the same disqualification as a dropped
    /// sample, arrived at from the other side.
    fn write(&mut self, batch: &[f32]) -> Result<(), String> {
        for &sample in batch {
            self.writer
                .write_sample(to_i16(sample))
                .map_err(|e| format!("Write failed on {}: {e}", self.path.display()))?;
        }
        self.samples += batch.len() as u64;
        Ok(())
    }

    fn dropped_so_far(&self, dropped: &AtomicU64) -> u64 {
        dropped.load(Ordering::Relaxed) - self.dropped_at_open
    }

    /// Close the WAV and report.
    ///
    /// Flushes the ring first, and flushes it by a count taken *once*, up front:
    /// everything captured before the Stop click belongs in the take, and nothing
    /// captured after it does. Draining until empty instead would race the still-
    /// live callback and staple audio from after the click onto the end; not
    /// draining at all would silently truncate every take by however far behind
    /// the writer happened to be. The count is the only version that is neither.
    ///
    /// This mirrors the dry-drain in [`Self::open`]: both ends of a take are cut
    /// at the click, not at a scheduling accident.
    fn close(mut self, cons: &mut SampleConsumer, sample_rate: u32, dropped: &AtomicU64) -> RecorderStatus {
        let pending = cons.occupied_len();
        let tail: Vec<f32> = (0..pending).filter_map(|_| cons.try_pop()).collect();
        if let Err(message) = self.write(&tail) {
            return RecorderStatus::Failed(message);
        }

        let dropped = self.dropped_so_far(dropped);
        let path = self.path;
        let samples = self.samples;
        // `finalize` is what writes the real lengths into the RIFF header, so
        // skipping it leaves a file every decoder reads as empty.
        match self.writer.finalize() {
            Ok(()) => {
                RecorderStatus::Finished(TakeReport {
                    path,
                    sample_rate,
                    samples,
                    dropped,
                })
            }
            Err(e) => RecorderStatus::Failed(format!("Cannot close {}: {e}", path.display())),
        }
    }
}

/// f32 → s16le.
///
/// The scale is 32768 rather than 32767 because that is the exact inverse of
/// `pulse_i16_to_f32`, which is how samples enter on the Pulse path — the path
/// this box actually records through, and the one whose `parec` invocation is
/// already the `parecord` line from `testdata/README.md`. So a Pulse take
/// round-trips bit-exact: the file holds the very integers the device sent, not a
/// rescaled approximation of them.
///
/// `i16::MIN` has magnitude 32768, so −1.0 lands exactly and only the +1.0 end
/// needs the clamp — reachable from an f32-format cpal device, never from Pulse.
fn to_i16(sample: f32) -> i16 {
    (sample * 32768.0)
        .round()
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

pub(super) fn start_recorder_worker(
    sample_rate: u32,
    mut cons: SampleConsumer,
    handle: RecorderHandle,
) -> AnalysisWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    let thread = thread::spawn(move || {
        let mut open: Option<OpenTake> = None;
        let mut batch: Vec<f32> = Vec::with_capacity(RECORDER_BATCH);

        while !stop_flag.load(Ordering::Relaxed) {
            sync_intent(&handle, &mut open, sample_rate, &mut cons);

            batch.clear();
            for _ in 0..RECORDER_BATCH {
                match cons.try_pop() {
                    Some(sample) => batch.push(sample),
                    None => break,
                }
            }
            if batch.is_empty() {
                thread::sleep(ANALYSIS_IDLE_SLEEP);
                continue;
            }

            // Idle drains and discards rather than parking on a full ring. Letting
            // it back up would have the callback's `try_push` fail and bump the drop
            // counter while nobody is recording — noise in the one number that has
            // to mean something.
            let Some(take) = open.as_mut() else { continue };

            match take.write(&batch) {
                Ok(()) => {
                    let status = RecorderStatus::Recording {
                        path:    take.path.clone(),
                        seconds: take.samples as f32 / sample_rate as f32,
                        dropped: take.dropped_so_far(&handle.dropped),
                    };
                    handle.publish(status);
                }
                Err(message) => {
                    // Drop the writer without finalizing: the file is already
                    // wrong, and a finalized header would dress it up as a whole
                    // take.
                    open = None;
                    handle.fail(message);
                }
            }
        }

        // The capture is going away — device switch, monitor toggle, or shutdown.
        // Any take in progress ends here rather than being carried over: the stream
        // it was recording no longer exists, and a take stitched across two devices
        // would not be one signal. `wanted` is deliberately left standing, so the
        // rebuilt capture's writer opens a fresh take and the UI's Record button
        // does not lie about what is running.
        if let Some(take) = open.take() {
            let status = take.close(&mut cons, sample_rate, &handle.dropped);
            handle.publish(status);
        }
    });

    AnalysisWorker { stop, thread }
}

/// Reconcile the take the writer holds against the one the UI wants.
///
/// Edge-triggered, so everything in here happens on a Record/Stop click rather
/// than per batch: in the steady state this is one uncontended lock and a compare.
fn sync_intent(
    handle: &RecorderHandle,
    open: &mut Option<OpenTake>,
    sample_rate: u32,
    cons: &mut SampleConsumer,
) {
    let wanted = handle.shared.lock().unwrap().wanted.clone();
    let holding = open.as_ref().map(|take| take.path.clone());
    if wanted == holding {
        return;
    }

    // Any change of intent closes what is open — a stop, or a new destination
    // asked for while a take is still running.
    if let Some(take) = open.take() {
        let status = take.close(cons, sample_rate, &handle.dropped);
        handle.publish(status);
    }

    let Some(path) = wanted else { return };
    match OpenTake::open(path, sample_rate, &handle.dropped, cons) {
        Ok(take) => {
            handle.publish(RecorderStatus::Recording {
                path:    take.path.clone(),
                seconds: 0.0,
                dropped: 0,
            });
            *open = Some(take);
        }
        Err(message) => handle.fail(message),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{
        Duration,
        Instant,
    };

    use super::super::pulse_i16_to_f32;
    use super::*;

    /// Spin until `done`, or fail loudly. The writer is a real thread on a 5 ms
    /// idle sleep, so the test has to wait for it — but a test that waits forever
    /// on a broken recorder is a hang, not a failure.
    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done() {
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("recorder never {what}");
    }

    /// [`to_i16`] claims a Pulse take round-trips bit-exact — that the WAV holds
    /// the very integers the device sent, not a rescaled likeness of them. That
    /// claim is the whole basis for calling a take *the signal the detector saw*,
    /// and it rests on a 32768-vs-32767 detail that is easy to get wrong and
    /// impossible to hear. So prove it rather than assert it, over every value an
    /// i16 can take.
    #[test]
    fn pulse_samples_round_trip_bit_exact() {
        for raw in i16::MIN..=i16::MAX {
            let round_tripped = to_i16(pulse_i16_to_f32(raw.to_le_bytes()));
            assert_eq!(round_tripped, raw, "i16 {raw} did not survive f32 → s16le");
        }
    }

    /// Drives the real writer thread through a real take — tap → ring → WAV on
    /// disk → read back — and asserts the one property a take exists for: the
    /// file holds exactly the samples that went into the tap, all of them, in
    /// order, unmodified.
    ///
    /// This is also the regression test for the tail. The writer's loop reconciles
    /// intent *before* it drains, so a `close` that did not flush the ring would
    /// still produce a perfectly valid, perfectly plausible WAV — just missing
    /// however much audio the writer happened to be behind by. It would pass
    /// every check except this one.
    #[test]
    fn a_take_holds_exactly_the_samples_that_went_in() {
        let sample_rate = 48_000;
        let path = std::env::temp_dir().join("fretboard_recorder_take.wav");
        let _ = std::fs::remove_file(&path);

        let handle = RecorderHandle::new();
        let (prod, cons) = recorder_ring(sample_rate);
        let mut tap = handle.tap(prod);
        let worker = start_recorder_worker(sample_rate, cons, handle.clone());

        // Every sample distinct, so a gap or a reorder cannot hide behind a
        // repeated value — and well inside i16, so nothing clips.
        let played: Vec<i16> = (0..12_000_i32).map(|i| (i - 6_000) as i16).collect();

        handle.start(path.clone());
        // Wait for the take to actually open before playing into it: samples
        // pushed before that are the pre-Record past, which `OpenTake::open`
        // deliberately throws away.
        wait_until("opened the take", || {
            matches!(handle.status(), RecorderStatus::Recording { .. })
        });

        for &sample in &played {
            tap.push(pulse_i16_to_f32(sample.to_le_bytes()));
        }

        // Stop immediately, with the ring still full of what we just pushed —
        // that is the interesting case, not a nicety. It is what a real Stop
        // click looks like.
        handle.stop();
        wait_until("finished the take", || {
            matches!(handle.status(), RecorderStatus::Finished(_))
        });
        worker.stop();

        let RecorderStatus::Finished(report) = handle.status() else {
            unreachable!("just waited for Finished")
        };
        assert!(report.is_evidence(), "take dropped {} samples", report.dropped);
        assert_eq!(
            report.sample_rate, sample_rate,
            "take must carry the device's rate"
        );
        assert_eq!(report.samples, played.len() as u64, "take is the wrong length");

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, sample_rate);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().bits_per_sample, RECORDER_BITS);

        let recorded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(recorded, played, "the take is not the signal that went in");

        std::fs::remove_file(&path).unwrap();
    }

    /// A take must be the signal the *device* produced, which is why the tap sits
    /// on the raw sample in the input callback and the gain is applied later, by
    /// the workers draining the other rings. Nothing in this module multiplies by
    /// anything — and that is exactly the kind of property that gets quietly
    /// undone by a well-meaning "normalise the take" patch, so pin it: whatever
    /// the tap is handed is what lands in the file.
    #[test]
    fn the_recorder_does_not_scale_what_it_is_given() {
        let quiet = pulse_i16_to_f32(1_000_i16.to_le_bytes());
        assert_eq!(to_i16(quiet), 1_000, "a quiet sample must stay quiet");

        // The same sample after a 4× gain would be a different integer. If a take
        // ever came out looking like this, the tap moved downstream of the slider.
        assert_eq!(
            to_i16(quiet * 4.0),
            4_000,
            "sanity: gain would be visible in the file"
        );
    }
}
