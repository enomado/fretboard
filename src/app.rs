#![cfg_attr(target_os = "android", allow(dead_code))]

mod controls;
mod fretboard_panel;
mod live_analysis;
mod persist;
mod resonator_panel;
mod scale_finder;
mod workspace;

use std::ops::Range;
use std::time::{
    Duration,
    Instant,
};

use eframe::egui::{
    self,
    Color32,
    Margin,
    Rangef,
    RichText,
    Ui,
};
use eframe::{
    CreationContext,
    Frame as AppFrame,
};

use crate::audio::{
    AudioEngine,
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
};
use crate::core_types::note::{
    ANote,
    Note,
};
use crate::core_types::pitch::{
    PCNote,
    PNote,
};
use crate::core_types::scale::{
    Degree,
    Scale,
};
use crate::core_types::scale_detect::ScaleFinderConfig;
use crate::core_types::tuning::{
    Fret,
    GString,
    Tuning,
};

const FRETBOARD_HEIGHT: f32 = 340.0;
const FRETBOARD_MARGIN_LEFT: f32 = 54.0;
const FRETBOARD_MARGIN_RIGHT: f32 = 24.0;
const FRETBOARD_MARGIN_TOP: f32 = 110.0;
const FRETBOARD_MARGIN_BOTTOM: f32 = 52.0;
const WINDOW_SIZE_PRESETS: [usize; 6] = [2048, 4096, 6144, 8192, 12288, 16384];
const FFT_SIZE_PRESETS: [usize; 4] = [4096, 8192, 16384, 32768];

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum TuningKind {
    Cello,
    StandardE,
    MinorThirds,
}

impl TuningKind {
    fn label(self) -> &'static str {
        match self {
            Self::Cello => "Cello (C-G-D-A)",
            Self::StandardE => "Guitar (E std)",
            Self::MinorThirds => "Minor thirds",
        }
    }

    fn to_tuning(self) -> Tuning {
        match self {
            Self::Cello => Tuning::cello(),
            Self::StandardE => Tuning::standart_e(),
            Self::MinorThirds => Tuning::minor_thirds(ANote::parse("D2").to_pitch()),
        }
    }
}

const ALL_TUNINGS: [TuningKind; 3] = [TuningKind::Cello, TuningKind::StandardE, TuningKind::MinorThirds];

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum ScaleKind {
    Major,
    Minor,
    BluesMinor,
    BluesMinorPentatonic,
    BluesMajor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
}

impl ScaleKind {
    fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::BluesMinor => "Blues minor",
            Self::BluesMinorPentatonic => "Blues minor pent.",
            Self::BluesMajor => "Blues major",
            Self::Dorian => "Dorian",
            Self::Phrygian => "Phrygian",
            Self::Lydian => "Lydian",
            Self::Mixolydian => "Mixolydian",
            Self::Locrian => "Locrian",
        }
    }

    fn to_scale(self, root: PCNote) -> Scale {
        match self {
            Self::Major => Scale::major(root),
            Self::Minor => Scale::minor(root),
            Self::BluesMinor => Scale::blues_minor(root),
            Self::BluesMinorPentatonic => Scale::blues_minor_pentatonic(root),
            Self::BluesMajor => Scale::blues_major(root),
            Self::Dorian => Scale::dorian(root),
            Self::Phrygian => Scale::phrygian(root),
            Self::Lydian => Scale::lydian(root),
            Self::Mixolydian => Scale::mixolydian(root),
            Self::Locrian => Scale::locrian(root),
        }
    }
}

const ALL_SCALES: [ScaleKind; 10] = [
    ScaleKind::Major,
    ScaleKind::Minor,
    ScaleKind::BluesMinor,
    ScaleKind::BluesMinorPentatonic,
    ScaleKind::BluesMajor,
    ScaleKind::Dorian,
    ScaleKind::Phrygian,
    ScaleKind::Lydian,
    ScaleKind::Mixolydian,
    ScaleKind::Locrian,
];

const ALL_ROOTS: [(Note, &str); 7] = [
    (Note::C, "C"),
    (Note::D, "D"),
    (Note::E, "E"),
    (Note::F, "F"),
    (Note::G, "G"),
    (Note::A, "A"),
    (Note::B, "B"),
];

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum LiveChartKind {
    Tuner,
    Fft,
    Spiral,
}

impl LiveChartKind {
    fn label(self) -> &'static str {
        match self {
            Self::Tuner => "Tuner",
            Self::Fft => "FFT",
            Self::Spiral => "Spiral",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum WorkspaceTab {
    Controls,
    FretboardControls,
    InputScope,
    ConfigGeneral,
    ConfigFft1,
    ConfigResonatorFft,
    LiveAnalysis,
    ScaleFinder,
    ResonatorBank,
    ResonatorSnail,
    ResonatorWaterfall,
    Fretboard,
}

impl WorkspaceTab {
    fn label(self) -> &'static str {
        match self {
            Self::Controls => "Controls",
            Self::FretboardControls => "Fretboard Controls",
            Self::InputScope => "Input Scope",
            Self::ConfigGeneral => "Config General",
            Self::ConfigFft1 => "Config FFT1",
            Self::ConfigResonatorFft => "Config Resonator FFT",
            Self::LiveAnalysis => "Live analysis",
            Self::ScaleFinder => "Scale Finder",
            Self::ResonatorBank => "Resonator Bank",
            Self::ResonatorSnail => "Resonator Snail",
            Self::ResonatorWaterfall => "Resonator Waterfall",
            Self::Fretboard => "Fretboard",
        }
    }
}

pub struct App {
    audio: AudioEngine,
    audio_inputs: Vec<AudioInputOption>,
    last_audio_input_refresh: Instant,
    tuning_kind: TuningKind,
    scale_kind: ScaleKind,
    root_note: Note,
    live_chart: LiveChartKind,
    test_note_midi: PNote,
    /// Конфиг Scale Finder: баланс методов + ширина окна (персистится).
    scale_finder: ScaleFinderConfig,
    /// Решалка Scale Finder: рантайм-окно chroma по времени (не персистится,
    /// тикается только пока панель видима). См. [`scale_finder::solver`].
    scale_solver: scale_finder::solver::ScaleSolver,
    workspace_tree: Option<egui_tiles::Tree<WorkspaceTab>>,
}

struct HoveredNote {
    string:    GString,
    fret:      Fret,
    note_name: ANote,
    degree:    Option<Degree>,
    center:    egui::Pos2,
    rect:      egui::Rect,
}

struct TunerTarget {
    string:       GString,
    fret:         Fret,
    note_name:    ANote,
    frequency_hz: f32,
    cents:        f32,
}

struct ResonatorTarget {
    string:   GString,
    fret:     Fret,
    strength: f32,
}

impl App {
    pub fn new(cc: &CreationContext) -> Self {
        crate::ui::theme::apply_theme(&cc.egui_ctx);
        let audio = AudioEngine::new();
        let audio_inputs = audio.available_inputs();

        let persisted = Self::load_persistent(cc);

        let mut app = Self {
            audio,
            audio_inputs,
            last_audio_input_refresh: Instant::now(),
            tuning_kind: TuningKind::Cello,
            scale_kind: ScaleKind::BluesMinor,
            root_note: Note::A,
            live_chart: LiveChartKind::Spiral,
            test_note_midi: PNote::new(24).unwrap(),
            scale_finder: ScaleFinderConfig::default(),
            scale_solver: scale_finder::solver::ScaleSolver::default(),
            workspace_tree: Some(workspace::default_workspace_tree()),
        };

        // Restore last session's preferences over the defaults built above.
        // Defaults stay intact for any field a stale RON file is missing.
        if let Some(state) = persisted {
            app.apply_persistent(state);
        }

        app
    }

    fn selected_input_kind(&self, selected_input_id: Option<&str>) -> AudioInputKind {
        if let Some(id) = selected_input_id {
            if id.starts_with("cpal-loopback::")
                || id.ends_with("@DEFAULT_MONITOR@")
                || id.ends_with(".monitor")
            {
                return AudioInputKind::System;
            }
        }

        selected_input_id
            .and_then(|id| self.audio_inputs.iter().find(|option| option.id == id))
            .map(|option| option.kind)
            .unwrap_or(AudioInputKind::Other)
    }

    fn refresh_audio_inputs(&mut self) {
        self.audio_inputs = self.audio.available_inputs();
        self.last_audio_input_refresh = Instant::now();
    }

    fn refresh_audio_inputs_if_stale(&mut self) {
        if self.last_audio_input_refresh.elapsed() >= Duration::from_secs(2) {
            self.refresh_audio_inputs();
        }
    }

    fn preferred_input_id(&self, kind: AudioInputKind) -> Option<String> {
        let preferred_pulse_id = match kind {
            AudioInputKind::Microphone => Some("pulse::@DEFAULT_SOURCE@"),
            AudioInputKind::System => Some("pulse::@DEFAULT_MONITOR@"),
            AudioInputKind::Other => None,
        };

        let concrete_pulse_microphone = (kind == AudioInputKind::Microphone).then(|| {
            self.audio_inputs.iter().find(|option| {
                option.kind == AudioInputKind::Microphone
                    && option.id.starts_with("pulse::")
                    && option.id != "pulse::@DEFAULT_SOURCE@"
            })
        });

        concrete_pulse_microphone
            .flatten()
            .or_else(|| {
                preferred_pulse_id.and_then(|preferred_id| {
                    self.audio_inputs.iter().find(|option| option.id == preferred_id)
                })
            })
            .or_else(|| {
                (kind == AudioInputKind::System)
                    .then(|| {
                        self.audio_inputs
                            .iter()
                            .find(|option| option.id.starts_with("cpal-loopback::"))
                    })
                    .flatten()
            })
            .or_else(|| self.audio_inputs.iter().find(|option| option.kind == kind))
            .or_else(|| self.audio_inputs.first())
            .map(|option| option.id.clone())
    }
}

pub fn create_app(cc: &CreationContext) -> App {
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
        let ctx = cc.egui_ctx.clone();
        ctx.set_pixels_per_point(1.75);
        subsecond::register_handler(std::sync::Arc::new(move || ctx.request_repaint()));
    }

    App::new(cc)
}

fn pill(ui: &mut Ui, label: &str, fg: Color32, bg: Color32) {
    eframe::egui::Frame::new()
        .fill(bg)
        .corner_radius(eframe::egui::CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(12.0).color(fg));
        });
}

fn audio_status_label(status: &AudioStatus, input_kind: AudioInputKind) -> String {
    match status {
        AudioStatus::Idle => format!("{} idle", input_source_label(input_kind)),
        AudioStatus::Listening => format!("Listening to {}", input_source_label(input_kind).to_lowercase()),
        AudioStatus::Error(message) => format!("Audio error: {message}"),
    }
}

fn audio_status_color(status: &AudioStatus) -> Color32 {
    match status {
        AudioStatus::Idle => Color32::from_rgb(154, 160, 168),
        AudioStatus::Listening => Color32::from_rgb(185, 194, 176),
        AudioStatus::Error(_) => Color32::from_rgb(210, 166, 136),
    }
}

fn cents_color(cents: f32) -> Color32 {
    if cents.abs() < 6.0 {
        Color32::from_rgb(182, 197, 164)
    } else if cents.abs() < 18.0 {
        Color32::from_rgb(206, 188, 151)
    } else {
        Color32::from_rgb(198, 146, 126)
    }
}

fn input_source_label(input_kind: AudioInputKind) -> &'static str {
    match input_kind {
        AudioInputKind::Microphone => "Microphone",
        AudioInputKind::System => "System audio",
        AudioInputKind::Other => "Audio input",
    }
}

fn input_level_label(input_kind: AudioInputKind) -> &'static str {
    match input_kind {
        AudioInputKind::Microphone => "Mic level",
        AudioInputKind::System => "System level",
        AudioInputKind::Other => "Input level",
    }
}

fn input_backend_label(selected_input_id: Option<&str>) -> &'static str {
    match selected_input_id {
        Some(id) if id.starts_with("pulse::") => "PulseAudio",
        Some(id) if id.starts_with("cpal-loopback::") => "WASAPI loopback",
        Some(id) if id.starts_with("cpal::") => "CPAL",
        Some(_) => "Custom",
        None => "None",
    }
}

fn input_source_debug_label(selected_input_id: Option<&str>) -> String {
    match selected_input_id {
        Some(id) => format!("{} • {}", input_backend_label(Some(id)), id),
        None => "No input selected".to_owned(),
    }
}

fn input_path_class_label(selected_input_id: Option<&str>) -> &'static str {
    match selected_input_id {
        Some(id) if id.starts_with("pulse::@DEFAULT_SOURCE@") => "Direct source",
        Some(id)
            if id.starts_with("pulse::")
                && (id.ends_with("@DEFAULT_MONITOR@") || id.ends_with(".monitor")) =>
        {
            "Monitor source"
        }
        Some(id) if id.starts_with("cpal-loopback::") => "Loopback source",
        Some(id) if id.starts_with("pulse::") => "Direct source",
        Some(id) if id.starts_with("cpal::") && is_compat_input_path(id) => "Compat path",
        Some(id) if id.starts_with("cpal::") => "Device path",
        Some(_) => "Custom path",
        None => "No input",
    }
}

fn input_path_detail(selected_input_id: Option<&str>) -> &'static str {
    match selected_input_id {
        Some(id) if id.starts_with("pulse::@DEFAULT_SOURCE@") => {
            "Server-chosen live source. Usually the safest low-friction path."
        }
        Some(id)
            if id.starts_with("pulse::")
                && (id.ends_with("@DEFAULT_MONITOR@") || id.ends_with(".monitor")) =>
        {
            "Loopback / monitor source exposed by the audio server."
        }
        Some(id) if id.starts_with("cpal-loopback::") => {
            "Default Windows output captured through WASAPI loopback."
        }
        Some(id) if id.starts_with("pulse::") => "Direct Pulse/PipeWire source path.",
        Some(id) if id.starts_with("cpal::") && is_compat_input_path(id) => {
            "Compatibility device. Often adds plugin buffering or resampling on Linux."
        }
        Some(id) if id.starts_with("cpal::") => "Direct host device via CPAL.",
        Some(_) => "Custom audio path.",
        None => "Choose an input to inspect the route.",
    }
}

fn is_compat_input_path(input_id: &str) -> bool {
    let lowered = input_id.to_lowercase();
    [
        "default",
        "pulse",
        "sysdefault",
        "front:",
        "surround",
        "plug",
        "dmix",
        "dsnoop",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn monitor_output_debug_label(output_name: Option<&str>, output_sample_rate: Option<u32>) -> String {
    match (output_name, output_sample_rate) {
        (Some(name), Some(rate)) => format!("{name} • {} Hz", rate),
        (Some(name), None) => format!("{name} • idle"),
        (None, Some(rate)) => format!("Unknown output • {} Hz", rate),
        (None, None) => "No active monitor output".to_owned(),
    }
}

fn output_has_bluetooth_risk(output_name: Option<&str>) -> bool {
    output_name.is_some_and(|name| {
        let lowered = name.to_lowercase();
        lowered.contains("bluez") || lowered.contains("bluetooth") || lowered.contains("a2dp")
    })
}

fn input_supports_monitor(selected_input_id: Option<&str>) -> bool {
    selected_input_id.is_some_and(|id| {
        !id.starts_with("cpal-loopback::") && !id.ends_with("@DEFAULT_MONITOR@") && !id.ends_with(".monitor")
    })
}

fn waiting_prompt(input_kind: AudioInputKind) -> &'static str {
    match input_kind {
        AudioInputKind::Microphone => "Play a single sustained note near the microphone",
        AudioInputKind::System => "Play audio on the system output or loopback device",
        AudioInputKind::Other => "Feed a signal into the selected input device",
    }
}

fn spectrum_color(value: f32) -> Color32 {
    let value = value.clamp(0.0, 1.0);
    let r = (96.0 + value * 70.0).round() as u8;
    let g = (88.0 + value * 82.0).round() as u8;
    let b = (82.0 + value * 56.0).round() as u8;
    Color32::from_rgb(r, g, b)
}

fn midi_to_frequency(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

fn frequency_to_midi(frequency_hz: f32) -> f32 {
    69.0 + 12.0 * (frequency_hz / 440.0).log2()
}

fn format_sample_count(value: usize) -> String {
    if value >= 1000 {
        format!("{:.1}k", value as f32 / 1000.0)
    } else {
        value.to_string()
    }
}

pub fn rangef_to_range(range: Rangef) -> Range<f32> {
    range.min..range.max
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut AppFrame) {
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        subsecond::call(|| self.render(ui));

        #[cfg(any(target_arch = "wasm32", target_os = "android"))]
        self.render(ui);
    }

    /// Called by eframe on its auto-save interval and on shutdown. Serializes a
    /// snapshot of preferences to RON (eframe's `set_value` uses `ron::ser`).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.snapshot_persistent());
    }
}
