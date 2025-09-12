use std::ops::Range;

use crate::tuning::{Fret, GString, Tuning};

pub enum FretConfig {
    Log,
    Linear,
}

pub struct Fretboard {
    pub screen_size_x: Range<f32>,
    pub screen_size_y: Range<f32>,
    pub config: FretConfig,
    pub tuning: Tuning,
    pub fret_range_show: Fret,
}

impl Fretboard {
    pub fn fret_pos(&self, n: Fret) -> f32 {
        match self.config {
            FretConfig::Log => fret_position_log(&self.screen_size_x, n, self.fret_range_show),
            FretConfig::Linear => {
                fret_position_linear(&self.screen_size_x, n, self.fret_range_show)
            }
        }
    }

    pub fn string_pos(&self, s: GString) -> f32 {
        string_position(
            &self.screen_size_y,
            s,
            (self.tuning.string_count() + 2) as u32,
        )
    }

    pub fn iter_frets(&self) -> impl Iterator<Item = Fret> {
        (1..self.fret_range_show.0 + 1).into_iter().map(Fret)
    }

    pub fn iter_strings(&self) -> impl Iterator<Item = GString> {
        (1..self.tuning.string_count() + 1).into_iter().map(GString)
    }
}

fn fret_position_log(range: &std::ops::Range<f32>, n: Fret, max_fret: Fret) -> f32 {
    // let scale_length = range.end - range.start + 300.;

    // / (1.0 - 2f32.powf(-(max_fret as f32 / 12.0)));

    let scale_length = (range.end - range.start) / (1.0 - 2f32.powf(-(max_fret.0 as f32 / 12.0)));

    range.start + scale_length - scale_length / 2f32.powf(n.0 as f32 / 12.0)
}

fn fret_position_linear(range: &std::ops::Range<f32>, n: Fret, max_fret: Fret) -> f32 {
    let span = range.end - range.start;
    range.start + span * (n.0 as f32 / max_fret.0 as f32)
}

fn string_position(range: &std::ops::Range<f32>, string_index: GString, string_count: u32) -> f32 {
    let span = range.end - range.start;
    // равномерно делим диапазон на string_count - 1 интервалов
    range.start + span * string_index.0 as f32 / (string_count.saturating_sub(1) as f32)
}
