mod controls;
mod fretboard_panel;
mod live_analysis;
mod workspace;

use std::ops::Range;

use eframe::egui::{
    self,
    Color32,
    Margin,
    Rangef,
    RichText,
    Ui,
    vec2,
};
use eframe::{
    CreationContext,
    Frame as AppFrame,
};

use crate::audio::{
    AnalysisSettings,
    AudioEngine,
    AudioInputKind,
    AudioInputOption,
    AudioStatus,
};
use crate::core_types::note::{
    ANote,
    Note,
};
use crate::core_types::pitch::PCNote;
use crate::core_types::scale::Scale;
use crate::core_types::tuning::Tuning;

const FRETBOARD_HEIGHT: f32 = 340.0;
const FRETBOARD_MARGIN_LEFT: f32 = 54.0;
const FRETBOARD_MARGIN_RIGHT: f32 = 24.0;
const FRETBOARD_MARGIN_TOP: f32 = 110.0;
const FRETBOARD_MARGIN_BOTTOM: f32 = 52.0;
const SPIRAL_PITCH_LABELS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
const WINDOW_SIZE_PRESETS: [usize; 6] = [2048, 4096, 6144, 8192, 12288, 16384];
const FFT_SIZE_PRESETS: [usize; 4] = [4096, 8192, 16384, 32768];

#[derive(Clone, Copy, PartialEq)]
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

    fn subtitle(self) -> &'static str {
        match self {
            Self::Cello => "Compact orchestral layout",
            Self::StandardE => "Classic six-string tuning",
            Self::MinorThirds => "Symmetric fretboard geometry",
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

#[derive(Clone, Copy, PartialEq)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceTab {
    Controls,
    LiveAnalysis,
    Fretboard,
}

impl WorkspaceTab {
    const ALL: [Self; 3] = [Self::Controls, Self::LiveAnalysis, Self::Fretboard];

    fn label(self) -> &'static str {
        match self {
            Self::Controls => "Controls",
            Self::LiveAnalysis => "Live analysis",
            Self::Fretboard => "Fretboard",
        }
    }
}

pub struct App {
    audio:          AudioEngine,
    audio_inputs:   Vec<AudioInputOption>,
    tuning_kind:    TuningKind,
    scale_kind:     ScaleKind,
    root_note:      Note,
    live_chart:     LiveChartKind,
    workspace_tree: Option<egui_tiles::Tree<WorkspaceTab>>,
}

struct HoveredNote {
    string:    usize,
    fret:      usize,
    note_name: String,
    degree:    Option<u8>,
    center:    egui::Pos2,
    rect:      egui::Rect,
}

struct TunerTarget {
    string:       usize,
    fret:         usize,
    note_name:    String,
    frequency_hz: f32,
    cents:        f32,
    degree:       Option<u8>,
}

impl App {
    pub fn new(cc: &CreationContext) -> Self {
        crate::ui::theme::apply_theme(&cc.egui_ctx);
        let audio = AudioEngine::new();
        let audio_inputs = audio.available_inputs();

        Self {
            audio,
            audio_inputs,
            tuning_kind: TuningKind::Cello,
            scale_kind: ScaleKind::BluesMinor,
            root_note: Note::A,
            live_chart: LiveChartKind::Spiral,
            workspace_tree: Some(workspace::default_workspace_tree()),
        }
    }

    fn root_label(&self) -> &'static str {
        ALL_ROOTS
            .iter()
            .find_map(|(note, label)| (*note == self.root_note).then_some(*label))
            .unwrap_or("?")
    }

    fn selected_input_kind(&self, selected_input_id: Option<&str>) -> AudioInputKind {
        selected_input_id
            .and_then(|id| self.audio_inputs.iter().find(|option| option.id == id))
            .map(|option| option.kind)
            .unwrap_or(AudioInputKind::Other)
    }

    fn preferred_input_id(&self, kind: AudioInputKind) -> Option<String> {
        self.audio_inputs
            .iter()
            .find(|option| option.kind == kind)
            .or_else(|| self.audio_inputs.first())
            .map(|option| option.id.clone())
    }
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

fn pitch_class_angle(pitch_class: usize) -> f32 {
    -std::f32::consts::FRAC_PI_2 + pitch_class as f32 * std::f32::consts::TAU / 12.0
}

fn spiral_point_fractional(
    center: egui::Pos2,
    inner_radius: f32,
    radius_step: f32,
    semitone_position: f32,
) -> egui::Pos2 {
    let angle = -std::f32::consts::FRAC_PI_2 + semitone_position * std::f32::consts::TAU / 12.0;
    let radius = inner_radius + semitone_position * radius_step;
    center + vec2(angle.cos(), angle.sin()) * radius
}

fn pitch_class_color(pitch_class: usize) -> Color32 {
    match pitch_class % 12 {
        0 => Color32::from_rgb(92, 230, 105),
        1 => Color32::from_rgb(104, 222, 170),
        2 => Color32::from_rgb(112, 204, 238),
        3 => Color32::from_rgb(122, 173, 255),
        4 => Color32::from_rgb(127, 138, 255),
        5 => Color32::from_rgb(164, 116, 246),
        6 => Color32::from_rgb(212, 98, 219),
        7 => Color32::from_rgb(236, 93, 168),
        8 => Color32::from_rgb(232, 110, 121),
        9 => Color32::from_rgb(239, 167, 102),
        10 => Color32::from_rgb(230, 203, 94),
        _ => Color32::from_rgb(156, 218, 115),
    }
}

fn spiral_note_color(pitch_class: usize, intensity: f32, alpha: u8) -> Color32 {
    let base = pitch_class_color(pitch_class);
    let glow = (40.0 + intensity * 120.0).round() as u8;
    Color32::from_rgba_unmultiplied(
        base.r().saturating_add(glow / 4),
        base.g().saturating_add(glow / 4),
        base.b().saturating_add(glow / 5),
        alpha,
    )
}

fn spiral_contrast_strengths(values: &[f32], settings: &AnalysisSettings) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }

    let peak = values.iter().copied().fold(0.0, f32::max);
    if peak <= 0.0 {
        return vec![0.0; values.len()];
    }

    let mean = values.iter().copied().sum::<f32>() / values.len() as f32;
    let gamma_norm = normalize_setting(settings.note_gamma, 0.35, 1.2);
    let spread_norm = normalize_setting(settings.note_spread, 0.15, 0.8);
    let threshold_floor = lerp(0.025, 0.11, gamma_norm);
    let threshold_ceiling = lerp(0.22, 0.36, gamma_norm);
    let threshold =
        (mean * lerp(1.15, 1.95, spread_norm) + threshold_floor).clamp(threshold_floor, threshold_ceiling);
    let scale = (1.0 - threshold).max(0.001);
    let mut strengths = vec![0.0; values.len()];

    for index in 0..values.len() {
        let intensity = values[index].clamp(0.0, 1.0);
        let normalized = ((intensity - threshold) / scale).clamp(0.0, 1.0);
        if normalized <= 0.0 {
            continue;
        }

        let left = values[index.saturating_sub(1)].clamp(0.0, 1.0);
        let right = values[(index + 1).min(values.len() - 1)].clamp(0.0, 1.0);
        let neighbor = left.max(right);
        let is_local_peak = intensity >= left && intensity >= right;
        let neighbor_guard = lerp(0.96, 0.84, spread_norm);
        let ridge = ((intensity - neighbor * neighbor_guard) / scale).clamp(0.0, 1.0);
        let focus = if is_local_peak {
            lerp(0.48, 0.78, 1.0 - spread_norm) + ridge * lerp(0.28, 0.62, 1.0 - spread_norm)
        } else {
            ridge * lerp(0.04, 0.18, spread_norm)
        };
        let emphasis = lerp(1.55, 2.65, gamma_norm);
        let emphasized = normalized.powf(emphasis) * focus;

        if emphasized > lerp(0.012, 0.05, gamma_norm) {
            strengths[index] = emphasized;
        }
    }

    strengths
}

fn normalize_setting(value: f32, min: f32, max: f32) -> f32 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t.clamp(0.0, 1.0)
}

fn waterfall_color(value: f32, age: f32) -> Color32 {
    let intensity = value.clamp(0.0, 1.0);
    let fade = (0.35 + age * 0.65).clamp(0.0, 1.0);
    let r = (34.0 + intensity * 138.0 * fade).round() as u8;
    let g = (42.0 + intensity * 120.0 * fade).round() as u8;
    let b = (52.0 + intensity * 92.0 * fade).round() as u8;
    Color32::from_rgb(r, g, b)
}

fn midi_to_frequency(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

fn frequency_to_midi(frequency_hz: f32) -> f32 {
    69.0 + 12.0 * (frequency_hz / 440.0).log2()
}

fn degree_suffix(degree: Option<u8>) -> String {
    degree
        .map(|value| format!(" • degree {}", value))
        .unwrap_or_default()
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
        #[cfg(not(target_arch = "wasm32"))]
        subsecond::call(|| self.render(ui));

        #[cfg(target_arch = "wasm32")]
        self.render(ui);
    }
}
