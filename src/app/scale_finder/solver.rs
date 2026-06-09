//! `ScaleSolver` — решалка Scale Finder как UI-объект (не канал/поток).
//!
//! Расчёт ансамбля дешёвый (свернуть 361→12 + 120 кандидатов × 4 метода = микро-
//! секунды) и по природе синхронен с перерисовкой, поэтому поток/ring-буфер ему
//! не нужны: лишняя межпоточная возня и lifecycle-баги ради семантики, которую
//! `draw` уже даёт даром. Решалке нужно ровно одно состояние — окно истории
//! chroma по ВРЕМЕНИ (до 20 c), которого нет в истории банка (та ~4 c).
//!
//! ИЗОЛЯЦИЯ: `tick` зовётся из `draw`, только пока панель видима. Закрыта —
//! не тикается, окно стынет; при следующем открытии старые кадры отсекаются по
//! возрасту и буфер наполняется заново (честно: нельзя интегрировать звук, что
//! не слушали).

use std::collections::VecDeque;
use std::time::Instant;

use crate::audio::{
    AnalysisSettings,
    TunerReading,
};
use crate::core_types::scale_detect::method_root::fold_bass_chroma;
use crate::core_types::scale_detect::{
    Chroma,
    fold_chroma,
};

/// Потолок хранения: 20 c окно + запас. Кадры старше — отбрасываются.
const HISTORY_MAX_AGE_SECS: f32 = 22.0;
/// Жёсткий потолок числа кадров — страховка от утечки при высоком FPS.
const HISTORY_MAX_FRAMES: usize = 2400;

/// Один кадр окна: момент, полная chroma и бас-взвешенная chroma (для метода C).
struct ChromaFrame {
    at:     Instant,
    chroma: Chroma,
    bass:   Chroma,
}

#[derive(Default)]
pub(crate) struct ScaleSolver {
    frames: VecDeque<ChromaFrame>,
}

impl ScaleSolver {
    /// Свернуть текущий резонаторный кадр и положить в окно; подрезать старое.
    /// Пустой спектр (банк ещё не зарядился) — пропускаем.
    pub(crate) fn tick(&mut self, now: Instant, reading: &TunerReading, settings: &AnalysisSettings) {
        if reading.resonator_spectrum.is_empty() {
            return;
        }
        let min_midi = settings.resonator.min_midi.as_u8() as usize;
        let bins = settings.resonator.bins;
        let chroma = fold_chroma(&reading.resonator_spectrum, min_midi, bins);
        let bass = fold_bass_chroma(&reading.resonator_spectrum, min_midi, bins);
        self.frames.push_back(ChromaFrame {
            at: now,
            chroma,
            bass,
        });
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        while self.frames.len() > HISTORY_MAX_FRAMES {
            self.frames.pop_front();
        }
        while let Some(front) = self.frames.front() {
            if now.duration_since(front.at).as_secs_f32() > HISTORY_MAX_AGE_SECS {
                self.frames.pop_front();
            } else {
                break;
            }
        }
    }

    /// Кадры chroma и bass за последние `seconds` (новейшие в конце буфера).
    pub(crate) fn window(&self, now: Instant, seconds: f32) -> (Vec<Chroma>, Vec<Chroma>) {
        let mut chroma = Vec::new();
        let mut bass = Vec::new();
        for frame in self.frames.iter().rev() {
            if now.duration_since(frame.at).as_secs_f32() > seconds {
                break;
            }
            chroma.push(frame.chroma);
            bass.push(frame.bass);
        }
        (chroma, bass)
    }

    /// Сколько секунд истории реально накоплено — для индикатора заполнения окна.
    pub(crate) fn captured_secs(&self, now: Instant) -> f32 {
        self.frames
            .front()
            .map_or(0.0, |front| now.duration_since(front.at).as_secs_f32())
    }
}
