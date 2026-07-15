//! Replay: a recorded take pushed back through the live path.
//!
//! The engine's fourth input. cpal and Pulse differ in where samples come from and
//! agree on everything after: rings, fan-out, the two analysis workers. Replay is
//! the same shape with the source swapped again — a thread reading a WAV instead of
//! a device callback. Nothing downstream knows the difference, and that is the whole
//! design: the line replay draws is the live line because it is drawn by the live
//! code, not by a second copy of it that could drift.
//!
//! Three properties carry that, and each is load-bearing:
//!
//! 1. **Real time, chunk by chunk.** Both planes gate their cadence on the wall
//!    clock (`core::ResonatorPipeline::last_publish`, `core::AnalysisPipeline::
//!    last_analysis`), so a take shovelled in at disk speed would be analysed once
//!    and never again. The sleep is not politeness — without it there is no line.
//!    See [`REPLAY_CHUNK`].
//!
//! 2. **The take's own rate, never resampled.** The pipelines are built per capture
//!    at whatever rate the source runs at, so a 44.1 kHz take is replayed as 44.1 —
//!    the rate it was captured at. Resampling it to match the device would be
//!    processing, banned by `testdata/README.md` for the same reason AGC is: it
//!    alters the quantity being measured.
//!
//! 3. **No recorder on this path.** A take is *what the violin played*; replay is a
//!    file that already exists. Recording it would produce a copy wearing the word
//!    "evidence", so the fan-out simply has no tap here (`InputFanout::recorder` is
//!    `None`) and the UI's Record button is dead while replay holds the capture.

use std::path::{
    Path,
    PathBuf,
};
use std::sync::atomic::{
    AtomicBool,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
};
use std::thread;
use std::time::Duration;

use super::{
    AnalysisWorker,
    InputFanout,
};
use crate::audio::types::{
    ReplayStatus,
    TakeOnDisk,
};

/// How much audio the source thread hands over per wake-up, in seconds.
///
/// 10 ms — a device callback's quantum, and the same one `core`'s test rig feeds at.
/// The value matters in both directions: much larger and the planes see the take in
/// lumps no real capture would deliver; much smaller and the thread spends its life
/// in the scheduler rather than pushing samples.
const REPLAY_CHUNK: f32 = 0.01;

/// Handle on replay, held by `AudioEngine` and cloned into `AudioContext`.
///
/// Mirrors `RecorderHandle`, and for the same reason: the status has to outlive the
/// capture that produced it. The UI asks "what happened to the take I played" after
/// the source thread is gone.
#[derive(Clone)]
pub(super) struct ReplayHandle {
    status: Arc<Mutex<ReplayStatus>>,
}

impl ReplayHandle {
    pub(super) fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(ReplayStatus::Idle)),
        }
    }

    pub(super) fn status(&self) -> ReplayStatus {
        self.status.lock().unwrap().clone()
    }

    pub(super) fn publish(&self, status: ReplayStatus) {
        *self.status.lock().unwrap() = status;
    }
}

/// A take read off disk, ready to be played into the engine.
///
/// Read whole, up front, deliberately: a take is seconds of mono s16 (a 35-second
/// one is about 3 MB), and buying that memory once means the source thread never
/// touches the filesystem while it is feeding a real-time path. A disk hiccup mid-
/// take would otherwise show up as a gap in the audio the detector sees — which is
/// to say, as a detector bug that is not one.
pub(super) struct LoadedTake {
    pub(super) path:        PathBuf,
    pub(super) samples:     Vec<f32>,
    pub(super) sample_rate: u32,
}

impl LoadedTake {
    pub(super) fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// Read a take off disk.
///
/// Fails loudly and specifically rather than coping. Every take this app records is
/// mono s16 (`recorder::OpenTake::open`), and so is every file in `testdata/`, so a
/// file that is not is not a take — it is some other audio the user pointed us at,
/// and silently downmixing or reinterpreting it would produce a line for a signal
/// nobody played. The corpus is the ground truth; guessing at its format is the one
/// thing that cannot be allowed to be approximate.
pub(super) fn load_take(path: &Path) -> Result<LoadedTake, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let spec = reader.spec();

    if spec.channels != 1 {
        return Err(format!(
            "{} has {} channels; takes are mono",
            path.display(),
            spec.channels
        ));
    }
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(format!(
            "{} is {:?}/{} bit; takes are s16",
            path.display(),
            spec.sample_format,
            spec.bits_per_sample
        ));
    }

    // 32768 mirrors `recorder::to_i16` exactly, so a take this app wrote round-trips
    // back to the very floats the device sent. The other readers of the corpus (the
    // bench, the engine's end-to-end tests) use the same divisor; a different one
    // here would mean replay analysed a slightly different signal than the tests do.
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

    if samples.is_empty() {
        return Err(format!("{} holds no audio", path.display()));
    }

    Ok(LoadedTake {
        path: path.to_owned(),
        samples,
        sample_rate: spec.sample_rate,
    })
}

/// What takes are sitting in `dir`, oldest name first.
///
/// Reads only each WAV's header — the length and rate are in the RIFF chunk, so this
/// costs a few bytes per file rather than the megabytes the samples would. It runs
/// off the UI thread's frame, and a corpus directory is a handful of files, so the
/// cost is a stat and an open apiece.
///
/// Anything that is not a readable mono s16 WAV is simply not listed. This is the one
/// place in the module that swallows an error rather than reporting it, and the reason
/// is that it is answering "what can I offer to play", not "is this file any good" —
/// an unrelated WAV someone dropped into `testdata/` is not an error to report to
/// anyone, it is just not a take. The file the user actually clicks Play on gets the
/// loud version of the check ([`load_take`]).
pub(super) fn list_takes(dir: &Path) -> Vec<TakeOnDisk> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // No corpus directory yet — nothing to replay, which is not a failure.
        return Vec::new();
    };

    let mut takes: Vec<TakeOnDisk> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        })
        .filter_map(|path| {
            let reader = hound::WavReader::open(&path).ok()?;
            let spec = reader.spec();
            if spec.channels != 1 || spec.sample_format != hound::SampleFormat::Int {
                return None;
            }
            Some(TakeOnDisk {
                samples: u64::from(reader.duration()),
                sample_rate: spec.sample_rate,
                path,
            })
        })
        .collect();

    // `read_dir` order is the filesystem's, which is arbitrary and unstable — the
    // list would reshuffle itself between frames and the user would click the wrong
    // take. By name is stable, and the corpus names sort meaningfully (`g_open_*`
    // together).
    takes.sort_by(|a, b| a.path.cmp(&b.path));
    takes
}

/// Play a loaded take into the fan-out at the rate it was recorded at.
///
/// Returns the same `AnalysisWorker` handle the other sources' threads use — replay
/// is stopped by the capture teardown like everything else, so a Stop click, a
/// device switch and app shutdown all end it through one path.
pub(super) fn start_replay_source(
    take: LoadedTake,
    mut fanout: InputFanout,
    handle: ReplayHandle,
) -> AnalysisWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    let thread = thread::spawn(move || {
        let sample_rate = take.sample_rate as f32;
        let chunk = (sample_rate * REPLAY_CHUNK) as usize;
        let total_seconds = take.seconds();

        for (index, block) in take.samples.chunks(chunk).enumerate() {
            // Checked per chunk, so a stop lands within 10 ms rather than at the end
            // of the take. The UI's Stop has to feel like a stop even 30 seconds in.
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }

            for &sample in block {
                fanout.push(sample);
            }

            // The audio clock, not a stopwatch: how far into the take the samples
            // just pushed reach. Counting chunks is exact where accumulating elapsed
            // wall time would drift — and this number is what the roll's playhead is
            // drawn at, so drift here would be a line that does not line up with the
            // audio that produced it.
            let seconds = ((index + 1) * chunk).min(take.samples.len()) as f32 / sample_rate;
            handle.publish(ReplayStatus::Playing {
                path: take.path.clone(),
                seconds,
                total_seconds,
            });

            // Wall time must keep up with audio time, which is the condition a real
            // capture always satisfies and the one both planes' cadence gates read.
            // Sleeping by the block's own length keeps the two equal.
            thread::sleep(Duration::from_secs_f32(block.len() as f32 / sample_rate));
        }

        // Ended on its own. The capture stays up (parked, pushing nothing) until the
        // user stops it: the line just drawn is what they are about to mark up, and
        // handing the input back to the microphone would write live frames over it.
        handle.publish(ReplayStatus::Finished { path: take.path });
    });

    AnalysisWorker { stop, thread }
}

#[cfg(test)]
mod tests {
    use ringbuf::traits::Consumer;

    use super::*;

    /// `load_take` is the only place a file becomes samples the detector will see, so
    /// the scale it reads at has to be the exact inverse of the one the recorder wrote
    /// at — otherwise replay analyses a quieter or louder signal than the take is, and
    /// every number that comes out is off by a constant nobody would think to look for.
    #[test]
    fn a_take_reads_back_at_the_scale_it_was_written_at() {
        let path = std::env::temp_dir().join("fretboard_replay_scale.wav");
        let spec = hound::WavSpec {
            channels:        1,
            sample_rate:     48_000,
            bits_per_sample: 16,
            sample_format:   hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        // The extremes plus a quiet sample: -1.0 lands exactly at i16::MIN, and the
        // 32768 divisor is what makes that true.
        for sample in [i16::MIN, -1_000, 0, 1_000, i16::MAX] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();

        let take = load_take(&path).unwrap();
        assert_eq!(take.sample_rate, 48_000);
        assert_eq!(take.samples[0], -1.0, "i16::MIN must read back as exactly -1.0");
        assert_eq!(take.samples[1], -1_000.0 / 32768.0);
        assert_eq!(take.samples[2], 0.0);
        assert_eq!(take.samples[3], 1_000.0 / 32768.0);

        std::fs::remove_file(&path).unwrap();
    }

    /// A file that is not a take must be refused by name rather than reinterpreted.
    /// Stereo read as mono would halve the pitch of everything in it — a bug that
    /// looks exactly like an octave error, in the one part of the app whose whole
    /// job is diagnosing octave errors.
    #[test]
    fn a_file_that_is_not_a_take_is_refused_not_guessed_at() {
        let path = std::env::temp_dir().join("fretboard_replay_stereo.wav");
        let spec = hound::WavSpec {
            channels:        2,
            sample_rate:     48_000,
            bits_per_sample: 16,
            sample_format:   hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..10 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();

        let Err(error) = load_take(&path) else {
            panic!("a stereo file was accepted as a take")
        };
        assert!(error.contains("channels"), "unhelpful refusal: {error}");

        std::fs::remove_file(&path).unwrap();
        assert!(
            load_take(&std::env::temp_dir().join("fretboard_replay_absent.wav")).is_err(),
            "a missing take must fail, not produce silence"
        );
    }

    /// Drives the real source thread through a real take and asserts the two
    /// properties replay exists for — the mirror image of the recorder's
    /// `a_take_holds_exactly_the_samples_that_went_in`:
    ///
    /// 1. **What comes out of the fan-out is the take, exactly.** The engine's whole
    ///    claim is that the line it draws for a take is the line the instrument gets;
    ///    that is worth nothing if the samples reaching the analysis rings are not the
    ///    ones in the file.
    ///
    /// 2. **It is not faster than real time.** This is the property that has no other
    ///    symptom: both planes gate their cadence on the wall clock, so a replay that
    ///    dumped the take at disk speed would still push every sample, still leave the
    ///    test above green — and produce a *single* analysed frame instead of a line.
    ///    Deleting the sleep would look correct everywhere except on screen.
    #[test]
    fn replay_hands_over_the_whole_take_at_the_rate_it_was_played() {
        let sample_rate = 48_000u32;
        // Short, but several chunks long — the loop's per-chunk bookkeeping (the
        // index → seconds arithmetic, the final partial block) only exists past the
        // first chunk. 0.2 s is 20 of them.
        let played: Vec<f32> = (0..9_600).map(|i| ((i % 200) as f32 - 100.0) / 32768.0).collect();
        let take = LoadedTake {
            path: PathBuf::from("in_memory_take.wav"),
            samples: played.clone(),
            sample_rate,
        };
        let audio_seconds = take.seconds();

        let (analysis_prod, mut analysis_cons) = super::super::analysis_ring(sample_rate);
        let (resonator_prod, _resonator_cons) = super::super::analysis_ring(sample_rate);
        let fanout = InputFanout {
            analysis:  analysis_prod,
            resonator: resonator_prod,
            monitor:   None,
            // Replay never records: the file already exists (see the module docs).
            recorder:  None,
        };

        let handle = ReplayHandle::new();
        let started = std::time::Instant::now();
        let source = start_replay_source(take, fanout, handle.clone());

        // Drain as it plays rather than after: the ring holds 0.5 s and the take is
        // 0.2 s, so it would fit — but draining alongside is what the real analysis
        // worker does, and a test that only works because the ring was big enough
        // would not be testing the path in use.
        let mut received: Vec<f32> = Vec::with_capacity(played.len());
        let deadline = started + Duration::from_secs(5);
        while received.len() < played.len() && std::time::Instant::now() < deadline {
            match analysis_cons.try_pop() {
                Some(sample) => received.push(sample),
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
        let elapsed = started.elapsed();
        source.stop();

        assert_eq!(
            received.len(),
            played.len(),
            "replay handed over {} of {} samples",
            received.len(),
            played.len()
        );
        assert_eq!(received, played, "what replay played is not what the take holds");
        assert_eq!(
            handle.status(),
            ReplayStatus::Finished {
                path: PathBuf::from("in_memory_take.wav"),
            },
            "a take that reached its end must report Finished — the UI unfreezes on it"
        );

        // The margin is generous because the assertion is one-directional: the bug
        // this guards against (no sleep) finishes 0.2 s of audio in microseconds, not
        // in 0.15 s. A loaded machine can only push this the safe way.
        assert!(
            elapsed.as_secs_f32() >= audio_seconds * 0.75,
            "replay pushed {audio_seconds:.2} s of audio in {:.3} s — faster than real time means \
             the planes' cadence gates see one frame instead of a line",
            elapsed.as_secs_f32()
        );
    }

    /// The corpus listing is what the panel offers to play, so a file that is not a
    /// take must not appear in it — and the ones that are must come back in a stable
    /// order, or the rows reshuffle under the user's cursor between frames.
    #[test]
    fn the_listing_offers_takes_and_only_takes() {
        let dir = std::env::temp_dir().join("fretboard_replay_listing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mono = hound::WavSpec {
            channels:        1,
            sample_rate:     48_000,
            bits_per_sample: 16,
            sample_format:   hound::SampleFormat::Int,
        };
        for name in ["b_take.wav", "a_take.wav"] {
            let mut writer = hound::WavWriter::create(dir.join(name), mono).unwrap();
            for _ in 0..4_800 {
                writer.write_sample(0i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        let mut stereo =
            hound::WavWriter::create(dir.join("c_stereo.wav"), hound::WavSpec { channels: 2, ..mono })
                .unwrap();
        // Both channels: a stereo WAV owing half a frame will not finalize.
        stereo.write_sample(0i16).unwrap();
        stereo.write_sample(0i16).unwrap();
        stereo.finalize().unwrap();
        std::fs::write(dir.join("notes.txt"), b"not audio at all").unwrap();

        let takes = list_takes(&dir);

        let names: Vec<String> = takes
            .iter()
            .map(|take| take.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["a_take.wav", "b_take.wav"],
            "listing is wrong or unsorted"
        );
        assert_eq!(takes[0].sample_rate, 48_000);
        assert_eq!(takes[0].samples, 4_800, "length must come off the header");
        assert_eq!(takes[0].seconds(), 0.1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
