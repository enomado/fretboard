//! RON persistence of UI + audio preferences between sessions.
//!
//! Storage goes through eframe's built-in `Storage` (enabled via the
//! `persistence` feature). On native eframe writes a RON file under the
//! platform config dir; on web it uses `localStorage`. `eframe::get_value` /
//! `set_value` serialize with RON, so "persist via RON" is satisfied without a
//! hand-rolled file path.
//!
//! Load is fail-soft: a parse failure (schema drift across versions, corrupt
//! file) yields `None` and the app falls back to its defaults. These are user
//! preferences, not invariant data — losing them on a breaking change is
//! acceptable and strictly better than refusing to start.

use eframe::CreationContext;

use super::{
    App,
    LiveChartKind,
    ScaleKind,
    TuningKind,
    WorkspaceTab,
};
use crate::audio::AnalysisSettings;
use crate::core_types::note::Note;
use crate::core_types::pitch::PNote;
use crate::core_types::scale_detect::ScaleFinderConfig;

/// Everything we carry across sessions. Owns a snapshot of the audio engine's
/// settings (the engine itself is rebuilt fresh each launch) plus the UI
/// selections and the docking layout.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PersistentState {
    tuning_kind:          TuningKind,
    scale_kind:           ScaleKind,
    root_note:            Note,
    live_chart:           LiveChartKind,
    test_note_midi:       usize,
    #[serde(default)]
    scale_finder:         ScaleFinderConfig,
    analysis_settings:    AnalysisSettings,
    input_gain:           f32,
    monitor_enabled:      bool,
    monitor_gain:         f32,
    selected_input_id:    Option<String>,
    workspace_tree:       egui_tiles::Tree<WorkspaceTab>,
}

impl App {
    /// Read persisted preferences from eframe storage, if any survived parsing.
    pub(super) fn load_persistent(cc: &CreationContext) -> Option<PersistentState> {
        let storage = cc.storage?;
        eframe::get_value(storage, eframe::APP_KEY)
    }

    /// Overwrite defaults with persisted preferences and push the audio-related
    /// ones into the freshly-built engine. Called once, right after `App::new`
    /// constructs the default state, so any field absent from an older RON file
    /// keeps its default.
    pub(super) fn apply_persistent(&mut self, state: PersistentState) {
        self.tuning_kind = state.tuning_kind;
        self.scale_kind = state.scale_kind;
        self.root_note = state.root_note;
        self.live_chart = state.live_chart;
        self.scale_finder = state.scale_finder;
        // Wire format stays a raw MIDI number; rebuild the newtype at the boundary.
        // A corrupt out-of-range value can't survive the contract — fail fast.
        self.test_note_midi = PNote::new(state.test_note_midi as u8).unwrap();
        self.workspace_tree = Some(state.workspace_tree);

        // The engine is the source of truth for these at runtime (the App reads
        // them back through getters), so restore them there rather than caching
        // copies on the App. A stale `selected_input_id` (device unplugged since
        // last session) is handled by the engine — it reports an error status
        // instead of panicking.
        self.audio.set_analysis_settings(state.analysis_settings);
        self.audio.set_input_gain(state.input_gain);
        self.audio.set_monitor_enabled(state.monitor_enabled);
        self.audio.set_monitor_gain(state.monitor_gain);
        self.audio.set_selected_input_id(state.selected_input_id);
    }

    /// Snapshot the current state for eframe to serialize to RON. The docking
    /// tree must exist by the time `save` runs (the renderer always puts it
    /// back after each frame), so `unwrap` is the contract, not a fallback.
    pub(super) fn snapshot_persistent(&self) -> PersistentState {
        PersistentState {
            tuning_kind:          self.tuning_kind,
            scale_kind:           self.scale_kind,
            root_note:            self.root_note,
            live_chart:           self.live_chart,
            test_note_midi:       self.test_note_midi.as_u8() as usize,
            scale_finder:         self.scale_finder,
            analysis_settings:    self.audio.analysis_settings(),
            input_gain:           self.audio.input_gain(),
            monitor_enabled:      self.audio.monitor_enabled(),
            monitor_gain:         self.audio.monitor_gain(),
            selected_input_id:    self.audio.selected_input_id(),
            workspace_tree:       self.workspace_tree.clone().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PersistentState;
    use crate::app::workspace::default_workspace_tree;
    use crate::audio::AnalysisSettings;
    use crate::core_types::note::Note;

    // Guards the RON wire contract: every persisted field (incl. the
    // egui_tiles tree and the audio settings) must round-trip. If a field
    // loses its serde derive this fails to compile or to deserialize.
    #[test]
    fn persistent_state_round_trips_through_ron() {
        let state = PersistentState {
            tuning_kind:          super::TuningKind::MinorThirds,
            scale_kind:           super::ScaleKind::Dorian,
            root_note:            Note::G,
            live_chart:           super::LiveChartKind::Fft,
            test_note_midi:       37,
            scale_finder:         crate::core_types::scale_detect::ScaleFinderConfig::default(),
            analysis_settings:    AnalysisSettings::default(),
            input_gain:           1.5,
            monitor_enabled:      true,
            monitor_gain:         0.25,
            selected_input_id:    Some("pulse::@DEFAULT_SOURCE@".to_owned()),
            workspace_tree:       default_workspace_tree(),
        };

        let ron = ron::ser::to_string(&state).unwrap();
        let back: PersistentState = ron::from_str(&ron).unwrap();

        assert_eq!(back.test_note_midi, 37);
        assert_eq!(back.root_note, Note::G);
        assert_eq!(back.selected_input_id.as_deref(), Some("pulse::@DEFAULT_SOURCE@"));
        assert!(back.monitor_enabled);
        assert_eq!(
            back.analysis_settings.resonator.min_midi,
            AnalysisSettings::default().resonator.min_midi
        );

        // The docking layout must survive too: same tile count and same set of
        // panes after the round-trip, not just a syntactically valid tree.
        let before = &state.workspace_tree;
        let after = &back.workspace_tree;
        assert_eq!(after.tiles.len(), before.tiles.len());
        let mut before_panes: Vec<_> = before.tiles.tiles().filter_map(pane_kind).collect();
        let mut after_panes: Vec<_> = after.tiles.tiles().filter_map(pane_kind).collect();
        before_panes.sort();
        after_panes.sort();
        assert_eq!(after_panes, before_panes);
        assert!(!after_panes.is_empty());
    }

    fn pane_kind(tile: &egui_tiles::Tile<super::WorkspaceTab>) -> Option<String> {
        match tile {
            egui_tiles::Tile::Pane(pane) => Some(format!("{:?}", pane.label())),
            egui_tiles::Tile::Container(_) => None,
        }
    }
}
