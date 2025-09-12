use std::ops::Range;

use crate::core_types::tuning::{Fret, GString, Tuning};

pub enum FretConfig {
    Log,
    Linear,
}

pub struct Fretboard {
    pub screen_size_x: Range<f32>,
    pub screen_size_y: Range<f32>,
    pub config: FretConfig,
    pub tuning: Tuning,
    pub fret_range_show: Range<Fret>,
}

impl Fretboard {
    pub fn fret_pos(&self, n: Fret) -> f32 {
        match self.config {
            FretConfig::Log => {
                fret_position_log_range(&self.screen_size_x, &self.fret_range_show, n)
            }
            FretConfig::Linear => {
                fret_position_linear_range(&self.screen_size_x, &self.fret_range_show, n)
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
        // (1..self.fret_range_show.0 + 1).into_iter().map(Fret)

        // dbg!(&self.fret_range_show);
        (self.fret_range_show.start.0..self.fret_range_show.end.0)
            .into_iter()
            .map(Fret)
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

pub fn fret_position_log_range(
    ui_range: &Range<f32>,    // координаты грифа
    fret_range: &Range<Fret>, // диапазон видимых ладов
    n: Fret,                  // какой лад рисуем
) -> f32 {
    let effective_end = fret_range.end.0 - fret_range.start.0 + 1;

    let scale_length =
        (ui_range.end - ui_range.start) / (1.0 - 2f32.powf(-(effective_end as f32 / 12.0)));

    let effective_fret = n.0 - fret_range.start.0 + 1;

    ui_range.start + scale_length - scale_length / 2f32.powf(effective_fret as f32 / 12.0)
}

// fn fret_position_linear_range(range: &Range<f32>, n: Fret, fret_range: &Range<Fret>) -> f32 {
//     let span = range.end - range.start;
//     let visible_frets = (fret_range.end.0 - fret_range.start.0) as f32;

//     // позиция относительно start_fret
//     let n_rel = (n.0 - fret_range.start.0) as f32;

//     range.start + span * (n_rel / visible_frets)
// }

pub fn fret_position_linear_range(ui_range: &Range<f32>, fret_range: &Range<Fret>, n: Fret) -> f32 {
    let visible = (fret_range.end.0 - fret_range.start.0 + 1) as f32;
    let n_eff = (n.0 - fret_range.start.0 + 1) as f32;

    ui_range.start + (ui_range.end - ui_range.start) * (n_eff / visible)
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
