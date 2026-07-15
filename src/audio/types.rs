use std::collections::VecDeque;
use std::path::PathBuf;

use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::PNote;

// Serialize/Deserialize so the wasm DSP web worker can ship readings back to the
// main thread (postcard over postMessage). Inert on native.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TunerReading {
    pub frequency_hz:          f32,
    pub note_name:             String,
    pub cents:                 f32,
    pub clarity:               f32,
    pub spectrum:              Vec<f32>,
    pub waterfall:             Vec<Vec<f32>>,
    pub note_spectrum:         Vec<f32>,
    pub note_waterfall:        Vec<Vec<f32>>,
    pub spiral_spectrum:       Vec<f32>,
    pub spiral_waterfall:      Vec<Vec<f32>>,
    pub resonator_spectrum:    Vec<f32>,
    pub resonator_waterfall:   Vec<Vec<f32>>,
    pub resonator_note_labels: Vec<String>,
    pub note_labels:           Vec<String>,
    /// The fast played-note prior: `(fractional_midi, strength)` of the harmonic
    /// fundamental the resonator bank hears right now, or `None` when the bank is
    /// quiet. Published at the bank's fast cadence (~16 ms). This is the raw bank
    /// reading, octave and all — [`Self::melody_pitch`] is the one to draw notes
    /// from.
    pub fast_pitch:            Option<(f32, f32)>,
    /// The played note for the melody panels (staff, pitch roll):
    /// `(fractional_midi, strength)`, or `None` when the bank is parked/quiet.
    ///
    /// The bank's fast fine pitch with its octave pinned by pYIN — see
    /// [`crate::audio::dsp::melody`] for why the two sources are married this way
    /// round. Published at the bank's ~16 ms cadence and re-stamped by the 40 ms
    /// pYIN path, so it never blanks.
    ///
    /// Do **not** draw the melody line from [`Self::frequency_hz`]: that is pYIN
    /// alone, which is pinned to its analysis window and cannot follow a note change
    /// in under ~128 ms (measured). It is the right source for the tuner and the
    /// fretboard, where a steady reading beats a prompt one.
    pub melody_pitch:          Option<(f32, f32)>,
    /// Monotonic count of detected note onsets (attacks). Consumed by the engine's
    /// own [`crate::audio::dsp::segmenter`] to split a re-bowed repeat of the same
    /// pitch instead of merging it into the held note; carried here because the two
    /// audio planes run at different cadences on different threads.
    pub onset_seq:             u64,
    /// The written line: the notes the engine has segmented out of the melody pitch,
    /// plus the one still sounding. Decided in [`crate::audio::dsp::segmenter`] at
    /// the bank's cadence on a **sample** clock — the staff panel draws this, it does
    /// not compute it.
    pub note_line:             NoteLine,
}

/// One finished note on the written line.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct StaffNote {
    pub midi:  i32,
    /// Deviation from equal temperament, cents. Kept per note so a written note's
    /// intonation stays visible after it has scrolled away from the playhead.
    pub cents: f32,
}

/// What the note segmenter has decided so far.
///
/// A plain snapshot, rebuilt each bank frame and carried on [`TunerReading`]: the
/// segmenter's own state (the held note's timers, the onset it last consumed) stays
/// in the engine, because those are the things that must not be driven by a UI frame.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct NoteLine {
    /// Finished notes, oldest → newest.
    pub history: Vec<StaffNote>,
    /// The note sounding right now — drawn emphasised at the playhead, not yet
    /// written (it may still turn out to be too short to count).
    pub current: Option<StaffNote>,
}

/// One published bank frame of the melody line, exactly as the engine decided it.
///
/// The unit the melody panels read *history* in, as opposed to [`TunerReading`],
/// which is the instant. All of a pitch roll's layers ride the **same** frame, so
/// they cannot drift apart: the line, the heat under it and the salience it was
/// decoded from are aligned 1:1 by construction rather than by three `push_back`s
/// next to each other agreeing to be. That alignment is the whole job of the layers
/// under the line — they are what the line is checked against by eye, and a column
/// from a different instant would be a lie rather than a wobble.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MelodyFrame {
    /// Delivery cursor: "have I seen this frame?".
    ///
    /// Monotonic for the life of the engine and pointedly **not** reset with the
    /// stream — a consumer holding a cursor across a device switch must never be
    /// handed a number it has already seen, or it would silently skip the new
    /// stream's first frames.
    pub seq:      u64,
    /// When it was played: seconds on the **sample clock** (`samples_seen /
    /// sample_rate`), the same clock the note segmenter's durations ride.
    ///
    /// This, and not [`Self::seq`], is the time axis. The bank's publish is gated on
    /// the *wall* clock (`last_publish.elapsed() >= update_ms`) and then fires on the
    /// first sample batch after the interval expires, so frames land ~16 ms apart
    /// **with jitter** — `seq × update_ms` would be just another wrong ruler, in the
    /// same family as counting UI frames. Unlike `seq` it restarts with the stream,
    /// because it describes the audio rather than the delivery.
    pub t:        f64,
    /// The melody line's note, or `None` for silence *or* a rejected octave slip —
    /// decided in `dsp::melody`, and deliberately the same `None` for both (see
    /// `dsp::segmenter` for why a rejected frame must read as a gap, not a change).
    pub pitch:    Option<f32>,
    /// The absolute input level the engine gated **this** frame on. Carried rather
    /// than read live so a panel fades a point by the level of the moment it is
    /// drawing, not of the moment it happens to be drawing at.
    pub level:    f32,
    /// The bank's heat column for this instant, raw: normalized to its own max, no
    /// octave decision, no silence gate. The ground truth, passed through untouched.
    pub heat:     Vec<f32>,
    /// What SWIPE′ *scored* that same column at, per bin — the evidence [`Self::pitch`] was
    /// decoded from. Same grid and same length as [`Self::heat`], so a panel swaps one for
    /// the other; `None` for a column with no energy at all.
    ///
    /// **It is not a second ground truth, and must not be offered as one.** `heat` is an
    /// independent witness — the physical spectrum, which disagreeing with the line is
    /// *information*. This curve is where the line came from, so it agrees with the line by
    /// construction: a mirror, not a witness. It answers "why did the decoder choose that",
    /// which `heat` cannot, and it answers nothing about whether the choice was right.
    ///
    /// Expect it to look *broad* and to read as a mush next to the spectrum's crisp
    /// partials. That is SWIPE′ being honest rather than the layer being broken: the h=1
    /// lobe alone spans 71 bins (~9 semitones at 8 bins/semitone), which is the same
    /// low-contrast curve that made `melody::SALIENCE_BETA` necessary in the first place.
    pub salience: Option<Vec<f32>>,
}

/// How much melody history the **engine** keeps for the panels, in seconds.
///
/// Short on purpose. The panels keep their own, much longer, view history; this only
/// has to bridge the gap between two of their reads, and two seconds is far more than
/// any plausible UI stall at ~240 KB per column layer. Keeping the panel's whole ~10 s
/// span here would cost 1.2 MB a layer and buy nothing: the bank parks when no panel is
/// asking, so there is never a backlog waiting to fill a freshly opened panel anyway.
///
/// Note the *per layer*: [`MelodyFrame::salience`] doubles the bill, and it is carried
/// unconditionally rather than gated on the pitch roll's toggle. That is a deliberate
/// trade and worth revisiting if it ever bites — a gate would mean the layer only starts
/// existing when switched on, so the ten seconds already on screen would be blank at the
/// moment you reach for it, which is precisely when the evidence is wanted. The obvious
/// escape (`request_resonator`'s grace-window pattern) buys the memory back at the price
/// of that blank, and the picture is worth more than 1.2 MB.
pub const MELODY_HISTORY_SECONDS: f64 = 2.0;

/// The melody line's recent past: a ring of [`MelodyFrame`]s trimmed by **time**.
///
/// One implementation, three users, deliberately — the engine's own history, the wasm
/// main thread's copy of it (the pipelines live in a worker, which can only post
/// deltas up), and the pitch roll's much longer view buffer. They keep different
/// spans, which is what [`Self::with_retention`] is for, but the rules about *what a
/// history is* — ordered, unique, self-healing across a stream restart — are written
/// once. Three hand-rolled `VecDeque`s is how the invariant below gets implemented
/// twice and forgotten the third time.
pub struct MelodyHistory {
    frames:         VecDeque<MelodyFrame>,
    retain_seconds: f64,
}

impl Default for MelodyHistory {
    /// The engine's retention — see [`MELODY_HISTORY_SECONDS`].
    fn default() -> Self {
        Self::with_retention(MELODY_HISTORY_SECONDS)
    }
}

impl MelodyHistory {
    /// A history keeping `retain_seconds` of **played audio** (not of wall time, and
    /// not a frame count).
    pub fn with_retention(retain_seconds: f64) -> Self {
        Self {
            frames: VecDeque::new(),
            retain_seconds,
        }
    }

    /// Append one frame, keeping both invariants.
    pub fn push(&mut self, frame: MelodyFrame) {
        // INVARIANT: strictly increasing in `t`. A new stream builds a fresh pipeline,
        // so its sample clock restarts at zero; dropping every frame not older than
        // the incoming one therefore clears a stale stream out **by construction**,
        // with no epoch counter to keep in sync with anything. While a stream runs the
        // clock only goes forward and this pops nothing.
        while self.frames.back().is_some_and(|f| f.t >= frame.t) {
            self.frames.pop_back();
        }
        let cutoff = frame.t - self.retain_seconds;
        self.frames.push_back(frame);
        // Trimmed by TIME, never by a frame count: `ResonatorSettings::update_ms` is
        // user-adjustable over a 10× range (8..80 ms), so a ring measured in frames is
        // not a span of seconds — which is the exact mistake this history exists to
        // undo, one level up.
        while self.frames.front().is_some_and(|f| f.t < cutoff) {
            self.frames.pop_front();
        }
    }

    /// Every frame newer than `after` (all of them for `None`), oldest → newest.
    ///
    /// A **cursor**, not a drain: the caller remembers where it got to, so any number
    /// of panels can read the same history independently. A drain would mean whichever
    /// panel polled first stole the frames from the rest — and a second consumer is
    /// expected (the staff's trail is the same shape).
    pub fn since(&self, after: Option<u64>) -> Vec<MelodyFrame> {
        self.frames
            .iter()
            .filter(|f| after.is_none_or(|seq| f.seq > seq))
            .cloned()
            .collect()
    }

    /// Oldest → newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &MelodyFrame> + ExactSizeIterator {
        self.frames.iter()
    }

    /// The frame at the playhead.
    pub fn newest(&self) -> Option<&MelodyFrame> {
        self.frames.back()
    }

    /// The frame at the far (left) edge.
    pub fn oldest(&self) -> Option<&MelodyFrame> {
        self.frames.front()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResonatorReading {
    pub spectrum:    Vec<f32>,
    pub waterfall:   Vec<Vec<f32>>,
    pub note_labels: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AudioStatus {
    Idle,
    Listening,
    Error(String),
}

/// What the take recorder is doing right now. The UI reads this every frame; the
/// writer thread (`audio::native::imp::recorder`) is the only thing that sets it.
#[derive(Clone, Debug, PartialEq)]
pub enum RecorderStatus {
    /// This build has no recorder. The browser engine captures through Web Audio
    /// and has no filesystem to put a take on, so the control is dead rather than
    /// quietly doing nothing — a Record button that silently records nowhere is
    /// worse than one that says it cannot.
    Unsupported,
    Idle,
    /// A take is open. `dropped` is live on purpose: if the writer starts losing
    /// samples you want to know while you can still stop and redo the take, not
    /// after you have played the whole thing (see [`TakeReport::dropped`]).
    Recording {
        path:    PathBuf,
        seconds: f32,
        dropped: u64,
    },
    /// The take that just finished. Held until the next one opens, so the UI has
    /// something to report after the button goes back to idle.
    Finished(TakeReport),
    Failed(String),
}

/// The outcome of a finished take.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeReport {
    pub path:        PathBuf,
    /// The rate the device actually gave us — *not* a fixed 44100. Writing what
    /// the device sent is what keeps the take equal to the signal the detector
    /// saw; resampling it to match the older takes would be processing, and the
    /// protocol in `testdata/README.md` forbids processing for exactly the reason
    /// it forbids AGC — it alters the quantity being measured.
    pub sample_rate: u32,
    pub samples:     u64,
    /// Samples the input callback captured but could not hand to the recorder,
    /// because the writer had fallen behind and the ring was full.
    ///
    /// **Non-zero disqualifies the take.** The audio callback must never block,
    /// so a full ring costs a sample rather than a stall — which means a take can
    /// come out with a hole in it while looking and sounding perfectly normal.
    /// Counting is the whole defence: it turns a silent corruption into a fact
    /// you can act on. A take with a hole is not the signal the detector saw, and
    /// therefore is not evidence.
    pub dropped:     u64,
}

impl TakeReport {
    pub fn seconds(&self) -> f32 {
        self.samples as f32 / self.sample_rate as f32
    }

    /// Whether this take can be used as ground truth. See [`Self::dropped`].
    pub fn is_evidence(&self) -> bool {
        self.dropped == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioInputKind {
    Microphone,
    System,
    Other,
}

#[derive(Clone, Debug)]
pub struct AudioInputOption {
    pub id:    String,
    pub label: String,
    pub kind:  AudioInputKind,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnalysisSettings {
    pub window_size:        usize,
    pub fft_size:           usize,
    pub min_frequency:      f32,
    pub max_frequency:      f32,
    /// Эталон A4 (камертон / concert pitch) в Гц. Задаёт стандарт строя:
    /// 440 — ISO, 442/443 — многие европейские оркестры, 430 — классика,
    /// 415 — барокко. Влияет на маппинг частота↔нота во всём движке.
    ///
    /// `serde(default)` — мягкая миграция: в RON прошлых версий поля нет, и без
    /// дефолта весь снимок настроек сбросился бы. Берём 440.0 (а НЕ `f32`-ноль).
    #[serde(default = "default_concert_pitch_hz")]
    pub concert_pitch_hz:   f32,
    pub spectrum_smoothing: usize,
    pub note_spread:        f32,
    pub spectrum_gamma:     f32,
    pub note_gamma:         f32,
    pub resonator:          ResonatorSettings,
    /// Как писать чёрные клавиши в подписях нот везде в UI: диезами (C#) или
    /// бемолями (Db). Глобальное свойство отображения — прокидывается во все
    /// генераторы подписей. `serde(default)` — старые снимки без поля грузятся
    /// как диезы (прежнее поведение подписей), а не падают на разборе.
    #[serde(default)]
    pub accidental:         AccidentalStyle,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResonatorSettings {
    pub min_midi:  PNote,
    pub max_midi:  PNote,
    pub bins:      usize,
    pub alpha:     f32,
    pub beta:      f32,
    pub gamma:     f32,
    pub history:   usize,
    pub update_ms: u64,
    pub power:     bool,
    /// Δφ instantaneous-frequency reassignment on the spiral: each bin's energy
    /// is placed at its measured true frequency (super-resolution) and a
    /// coherence gate suppresses the negative-frequency image / noise. Turning it
    /// off falls back to plain per-bin magnitude at the bin's nominal pitch — the
    /// original behaviour, kept as a safety net.
    ///
    /// `serde(default)` — soft migration: configs saved before this field existed
    /// load with reassignment on (the intended default), not `bool`'s `false`.
    #[serde(default = "default_reassign")]
    pub reassign:  bool,
}

fn default_reassign() -> bool {
    true
}

const MIN_WINDOW_SIZE: usize = 2048;
const MAX_WINDOW_SIZE: usize = 16384;
const MIN_FFT_SIZE: usize = 4096;
const MAX_FFT_SIZE: usize = 32768;
/// Low edge of the **spectrum display's** frequency axis — the floor `min_frequency`
/// (a view setting, see `dsp::spectrum`) may be dragged down to.
///
/// Deliberately *not* `dsp::pitch::LOWEST_TRACKED_FREQUENCY`, though it carried that
/// exact name and value until the tracker's floor moved up to C1. Two different
/// questions were sharing one number by coincidence: how low the pitch tracker hunts
/// for a fundamental, and how low the user may scroll the spectrum. Only the first
/// one moved — you can still *look* at 16 Hz, there is simply no longer a YIN lag
/// search down there.
const LOWEST_DISPLAYED_FREQUENCY: f32 = 16.0;

fn default_concert_pitch_hz() -> f32 {
    440.0
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            window_size:        6144,
            fft_size:           16384,
            min_frequency:      16.0,
            max_frequency:      2_000.0,
            concert_pitch_hz:   440.0,
            spectrum_smoothing: 1,
            note_spread:        0.35,
            spectrum_gamma:     0.58,
            note_gamma:         0.72,
            resonator:          ResonatorSettings::default(),
            accidental:         AccidentalStyle::default(),
        }
    }
}

impl Default for ResonatorSettings {
    fn default() -> Self {
        Self {
            min_midi:  PNote::new(12).unwrap(),
            // C8. The bank IS the melody line's pitch (see `dsp::melody`), so this is
            // the ceiling on what the staff and pitch roll can hear — it has to clear
            // the violin's top (E7 = 2637 Hz), not just the guitar's. Costs
            // resonators: the bank is per-sample IIR, so the span is a direct CPU
            // dial, and it stays user-adjustable for weak devices.
            max_midi:  PNote::new(108).unwrap(),
            bins:      5,
            alpha:     1.0,
            beta:      1.0,
            gamma:     0.72,
            history:   52,
            update_ms: 16,
            power:     false,
            reassign:  true,
        }
    }
}

impl AnalysisSettings {
    pub(crate) fn sanitized(mut self) -> Self {
        self.window_size = self.window_size.clamp(MIN_WINDOW_SIZE, MAX_WINDOW_SIZE);
        let min_fft_for_window = self
            .window_size
            .next_power_of_two()
            .clamp(MIN_FFT_SIZE, MAX_FFT_SIZE);
        self.fft_size = self
            .fft_size
            .max(MIN_FFT_SIZE)
            .next_power_of_two()
            .clamp(min_fft_for_window, MAX_FFT_SIZE);
        self.min_frequency = self.min_frequency.clamp(LOWEST_DISPLAYED_FREQUENCY, 1_200.0);
        self.max_frequency = self.max_frequency.clamp(120.0, 4_000.0);
        if self.max_frequency <= self.min_frequency + 40.0 {
            self.max_frequency = (self.min_frequency + 40.0).clamp(120.0, 4_000.0);
        }
        // Камертон: нижняя граница 400 Гц (чуть ниже барочного 415 — под
        // верди-строй и исторические низкие диапазоны), верхняя 466.16 (A#4).
        // Покрывает все академические стандарты (430/440/442/443/444) и не
        // даёт уехать в бессмыслицу.
        self.concert_pitch_hz = self.concert_pitch_hz.clamp(400.0, 466.0);
        self.spectrum_smoothing = self.spectrum_smoothing.min(4);
        self.note_spread = self.note_spread.clamp(0.15, 0.8);
        self.spectrum_gamma = self.spectrum_gamma.clamp(0.35, 1.2);
        self.note_gamma = self.note_gamma.clamp(0.35, 1.2);
        self.resonator = self.resonator.sanitized();
        self
    }
}

impl ResonatorSettings {
    pub(crate) fn sanitized(mut self) -> Self {
        // Clamp in raw-MIDI space (the newtype has no arithmetic), keeping the
        // invariant max ≥ min + 6, then rebuild the validated `PNote`s. The
        // bounds stay inside 0..=127, so `new` can't fail here.
        let min = self.min_midi.as_u8().clamp(12, 84);
        let mut max = self.max_midi.as_u8().clamp(24, 108);
        if max <= min + 6 {
            max = (min + 6).clamp(24, 108);
        }
        self.min_midi = PNote::new(min).unwrap();
        self.max_midi = PNote::new(max).unwrap();
        self.bins = self.bins.clamp(1, 12);
        // Низ 0.001 (а не 0.05): меньшая alpha = длиннее тау = «звон» резонатора;
        // именно нижняя декада даёт слышимый/видимый отклик, верх после
        // нормализации почти неразличим.
        self.alpha = self.alpha.clamp(0.001, 12.0);
        self.beta = self.beta.clamp(0.001, 12.0);
        self.gamma = self.gamma.clamp(0.15, 2.4);
        self.history = self.history.clamp(8, 240);
        self.update_ms = self.update_ms.clamp(8, 80);
        self
    }
}

// ----------------------------------------------------------------------------
// Drone player — целевой инструмент для занятий музыкой: тянет ноту/аккорд
// непрерывно, ритмично пульсирует или арпеджирует набор нот. Состояние живёт в
// движке (источник истины); UI читает/правит/пишет его целиком, как настройки
// анализа. Синтез — нативный (см. `audio::native`), wasm держит копию инертно.
// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DroneMode {
    /// Все ноты набора звучат непрерывно (классический дрон/аккорд).
    Sustained,
    /// Весь набор пульсирует в такт: вкл `pulse_duty` доли удара, потом тишина.
    Pulse,
    /// Ноты набора играются по очереди (маленький арпеджиатор) на каждый удар.
    Arp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
}

/// Голос дрона — форма волны/тембр. Синтез аддитивный/субтрактивный прямо в
/// колбэке (см. `audio::native`), без сэмплов. Каждый тембр нормирован к пику
/// ≈1, чтобы громкость не прыгала при переключении; `brightness` управляет
/// яркостью верхних гармоник внутри каждого тембра.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Timbre {
    /// Чистый тон с лёгкими обертонами — исходный голос дрона (дефолт).
    #[default]
    Sine,
    /// Смычковая струна: пилообразный спектр + лёгкое вибрато.
    Violin,
    /// Орган/драубары: фундамент + октавы, тёплый, без затухания.
    Organ,
    /// Субтрактивный синт: яркая пила со «срезом» по brightness.
    Synth,
    /// Электропиано: удар с перкуссивным затуханием (как зажатая клавиша).
    EPiano,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DroneState {
    /// Набор звучащих нот. Инвариант после `sanitized`: отсортирован, без
    /// дублей, длина ≤ [`DRONE_MAX_NOTES`]. Пустой набор = тишина (валидно).
    pub notes:        Vec<PNote>,
    /// Мастер-громкость дрона, 0..=1.
    pub gain:         f32,
    /// Эталон A4 (камертон). UI держит его в синхроне с настройками анализа,
    /// чтобы дрон звучал по тому же строю, что и тюнер. Частота ноты считается
    /// в реалтайме из этого значения, поэтому смена камертона слышна сразу.
    pub reference_hz: f32,
    pub mode:         DroneMode,
    /// Темп пульса/арпеджио в ударах в минуту (один удар = один шаг).
    pub bpm:          f32,
    /// Доля удара, в которой звук включён в режиме [`DroneMode::Pulse`], 0..=1.
    pub pulse_duty:   f32,
    /// Доля шага, в которой звучит нота в режиме [`DroneMode::Arp`], 0..=1.
    pub arp_gate:     f32,
    pub arp_pattern:  ArpPattern,
    /// Тембр: 0 = почти чистая синусоида, 1 = ярко, с обертонами.
    pub brightness:   f32,
    /// Голос/форма волны. `#[serde(default)]` — старые снимки без поля грузятся
    /// как [`Timbre::Sine`] (прежнее звучание), без падения разбора.
    #[serde(default)]
    pub timbre:       Timbre,
}

/// Потолок полифонии дрона. Держим набор компактным — это инструмент для
/// занятий (дрон/аккорд/арпеджио), а не сэмплер; заодно ограничивает сумму
/// амплитуд в реалтайм-колбэке.
pub const DRONE_MAX_NOTES: usize = 12;

impl Default for DroneState {
    fn default() -> Self {
        Self {
            // A2 — низкий, спокойный референс-дрон по умолчанию.
            notes:        vec![PNote::new(45).unwrap()],
            gain:         0.4,
            reference_hz: 440.0,
            mode:         DroneMode::Sustained,
            bpm:          80.0,
            pulse_duty:   0.5,
            arp_gate:     0.6,
            arp_pattern:  ArpPattern::Up,
            brightness:   0.4,
            timbre:       Timbre::Sine,
        }
    }
}

impl DroneState {
    pub fn sanitized(mut self) -> Self {
        // Ноты: отбрасываем дубли и сортируем (арпеджио идёт по высоте),
        // затем подрезаем до потолка полифонии.
        self.notes.sort();
        self.notes.dedup();
        self.notes.truncate(DRONE_MAX_NOTES);
        self.gain = self.gain.clamp(0.0, 1.0);
        // Камертон — те же границы, что у `AnalysisSettings` (400..466).
        self.reference_hz = self.reference_hz.clamp(400.0, 466.0);
        self.bpm = self.bpm.clamp(20.0, 300.0);
        self.pulse_duty = self.pulse_duty.clamp(0.05, 0.95);
        self.arp_gate = self.arp_gate.clamp(0.05, 1.0);
        self.brightness = self.brightness.clamp(0.0, 1.0);
        self
    }

    /// Переключить присутствие ноты в наборе (для тумблер-клавиатуры в UI).
    /// Сохраняет инвариант сортировки/уникальности.
    pub fn toggle_note(&mut self, note: PNote) {
        match self.notes.binary_search(&note) {
            Ok(idx) => {
                self.notes.remove(idx);
            }
            Err(idx) => {
                if self.notes.len() < DRONE_MAX_NOTES {
                    self.notes.insert(idx, note);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_settings_are_sanitized() {
        let settings = AnalysisSettings {
            window_size:        500,
            fft_size:           1_000,
            min_frequency:      900.0,
            max_frequency:      920.0,
            concert_pitch_hz:   900.0,
            spectrum_smoothing: 12,
            note_spread:        0.01,
            spectrum_gamma:     0.01,
            note_gamma:         9.0,
            resonator:          ResonatorSettings {
                min_midi:  PNote::new(10).unwrap(),
                max_midi:  PNote::new(11).unwrap(),
                bins:      99,
                alpha:     0.01,
                beta:      9.0,
                gamma:     9.0,
                history:   999,
                update_ms: 1,
                power:     false,
                reassign:  true,
            },
            accidental:         AccidentalStyle::Sharps,
        }
        .sanitized();

        assert!(settings.window_size >= MIN_WINDOW_SIZE);
        assert!(settings.fft_size >= settings.window_size.next_power_of_two());
        assert!(settings.max_frequency > settings.min_frequency);
        assert!((400.0..=466.0).contains(&settings.concert_pitch_hz));
        assert!(settings.spectrum_smoothing <= 4);
        assert!((0.15..=0.8).contains(&settings.note_spread));
        assert!(settings.resonator.max_midi > settings.resonator.min_midi);
        assert!((1..=12).contains(&settings.resonator.bins));
        assert!((0.001..=12.0).contains(&settings.resonator.alpha));
        assert!((0.001..=12.0).contains(&settings.resonator.beta));
        assert!((0.15..=2.4).contains(&settings.resonator.gamma));
        assert!((8..=240).contains(&settings.resonator.history));
        assert!((8..=80).contains(&settings.resonator.update_ms));
    }

    #[test]
    fn drone_sanitized_enforces_invariants() {
        let dirty = DroneState {
            // Дубли + не по порядку + сверх потолка полифонии.
            notes:        (0..30u8).rev().map(|i| PNote::new(40 + i).unwrap()).collect(),
            gain:         5.0,
            reference_hz: 1000.0,
            mode:         DroneMode::Arp,
            bpm:          5000.0,
            pulse_duty:   2.0,
            arp_gate:     -1.0,
            arp_pattern:  ArpPattern::UpDown,
            brightness:   9.0,
            timbre:       Timbre::Violin,
        };
        let clean = dirty.sanitized();
        assert!(clean.notes.len() <= DRONE_MAX_NOTES);
        assert!(clean.notes.windows(2).all(|w| w[0] < w[1]), "sorted + deduped");
        assert!((0.0..=1.0).contains(&clean.gain));
        assert!((400.0..=466.0).contains(&clean.reference_hz));
        assert!((20.0..=300.0).contains(&clean.bpm));
        assert!((0.05..=0.95).contains(&clean.pulse_duty));
        assert!((0.05..=1.0).contains(&clean.arp_gate));
        assert!((0.0..=1.0).contains(&clean.brightness));
    }

    #[test]
    fn drone_toggle_note_adds_and_removes_keeping_order() {
        let mut drone = DroneState {
            notes: Vec::new(),
            ..DroneState::default()
        };
        let a = PNote::new(57).unwrap();
        let b = PNote::new(45).unwrap();
        drone.toggle_note(a);
        drone.toggle_note(b);
        assert_eq!(drone.notes, vec![b, a], "kept sorted on insert");
        drone.toggle_note(a);
        assert_eq!(drone.notes, vec![b], "second toggle removes");
    }

    #[test]
    fn drone_toggle_note_respects_polyphony_cap() {
        let mut drone = DroneState {
            notes: Vec::new(),
            ..DroneState::default()
        };
        // Пытаемся набить больше потолка — лишние тихо отбрасываются.
        for midi in 36..36 + (DRONE_MAX_NOTES as u8 + 5) {
            drone.toggle_note(PNote::new(midi).unwrap());
        }
        assert_eq!(drone.notes.len(), DRONE_MAX_NOTES);
    }

    #[test]
    fn missing_concert_pitch_in_old_ron_defaults_to_a440() {
        // Снимок до появления камертона: поля concert_pitch_hz нет. Мягкая
        // миграция должна подставить 440.0, а не уронить разбор всего снимка.
        let ron = ron::ser::to_string(&AnalysisSettings::default()).unwrap();
        let legacy = ron.replace(",concert_pitch_hz:440.0", "");
        assert!(
            !legacy.contains("concert_pitch_hz"),
            "field must be absent for the test to be meaningful"
        );

        let restored: AnalysisSettings = ron::from_str(&legacy).unwrap();
        assert_eq!(restored.concert_pitch_hz, 440.0);
    }
}
