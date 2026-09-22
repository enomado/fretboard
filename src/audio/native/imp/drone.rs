//! Drone: the sustained reference tone(s) the player tunes and practises against.
//!
//! [`DroneSynth`] lives inside the drone output stream's realtime callback and adopts
//! the UI's [`DroneState`] snapshot every block (see
//! `AudioContext::build_drone_stream`); voices keep their phase across snapshots so a
//! change of notes or timbre mid-play does not click.

use crate::audio::sample_rate::SampleRate;
use crate::audio::types::{
    ArpPattern,
    DroneMode,
    DroneState,
    Timbre,
};
use crate::core_types::pitch::Midi;

/// Общий потолок громкости дрона после суммирования голосов (до мастер-гейна
/// из [`DroneState`]). Держит даже плотный аккорд в безопасном пределе.
const DRONE_OUTPUT_GAIN: f32 = 0.5;
/// Время сглаживания вкл/выкл голоса (атака/спад), сек. Убирает щелчки при
/// смене нот, пульсе и шагах арпеджио, не размазывая ритм.
const DRONE_RAMP_SECONDS: f32 = 0.006;
/// Вибрато смычка ([`Timbre::Violin`]): частота LFO и глубина (доля частоты).
/// Тонкое — для тепла, не для эффекта.
const DRONE_VIBRATO_HZ: f32 = 5.5;
const DRONE_VIBRATO_DEPTH: f32 = 0.004; // ±0.4 % высоты
/// Перкуссивное затухание удара ([`Timbre::EPiano`]): e-fold за столько секунд.
/// Зажатая клавиша затухает в тишину, как у настоящего пиано.
const DRONE_PLUCK_DECAY_SECONDS: f32 = 2.2;

// ------------------------------------------------------------------
// DroneSynth: реалтайм-синтез дрона. Живёт внутри колбэка дрон-стрима,
// держит фазу/амплитуду каждого голоса между блоками. Параметры берёт из
// снимка [`DroneState`] через `adopt`; ничего не аллоцирует на горячем пути,
// кроме `notes` (обновляется только в `adopt`).
// ------------------------------------------------------------------
pub(super) struct DroneSynth {
    sample_rate:      SampleRate,
    // Параметры (снимок DroneState, по одному полю чтобы не лочить в колбэке).
    notes:            Vec<u8>, // отсортированы по высоте (инвариант DroneState)
    gain:             f32,
    mode:             DroneMode,
    samples_per_step: f32, // длина удара в сэмплах = sr*60/bpm
    pulse_duty:       f32,
    arp_gate:         f32,
    arp_pattern:      ArpPattern,
    brightness:       f32,
    timbre:           Timbre,
    // Частота каждой MIDI-ноты при текущем камертоне (пересчёт в `adopt`).
    freq:             [f32; 128],
    // Живое состояние голосов.
    phase:            [f32; 128],  // фаза в циклах, 0..1
    amp:              [f32; 128],  // сглаженный гейт голоса (атака/спад), 0..1
    env:              [f32; 128],  // перкуссивная огибающая удара (EPiano), 0..1
    was_on:           [bool; 128], // гейт на прошлом сэмпле — для детекта удара
    clock:            f64,         // счётчик сэмплов для секвенсора
    lfo_phase:        f32,         // фаза LFO вибрато, 0..1
    ramp_k:           f32,         // шаг сглаживания гейта за сэмпл
    pluck_decay:      f32,         // множитель затухания env за сэмпл (EPiano)
}

impl DroneSynth {
    pub(super) fn new(sample_rate: SampleRate) -> Self {
        let mut synth = Self {
            sample_rate,
            notes: Vec::new(),
            gain: 0.0,
            mode: DroneMode::Sustained,
            samples_per_step: sample_rate.hz(), // = 60 bpm до первого adopt
            pulse_duty: 0.5,
            arp_gate: 0.6,
            arp_pattern: ArpPattern::Up,
            brightness: 0.4,
            timbre: Timbre::Sine,
            freq: [0.0; 128],
            phase: [0.0; 128],
            amp: [0.0; 128],
            env: [0.0; 128],
            was_on: [false; 128],
            clock: 0.0,
            lfo_phase: 0.0,
            // Экспоненциальное сглаживание с постоянной времени DRONE_RAMP_SECONDS.
            ramp_k: (1.0 / (DRONE_RAMP_SECONDS * sample_rate.hz())).clamp(0.0, 1.0),
            // Экспонента затухания удара: amp *= e^(-1/(τ·sr)) каждый сэмпл.
            pluck_decay: (-1.0 / (DRONE_PLUCK_DECAY_SECONDS * sample_rate.hz())).exp(),
        };
        synth.adopt(&DroneState::default());
        synth
    }

    /// Подхватить новый снимок состояния. Фаза/амплитуда/часы НЕ сбрасываются
    /// — параметры можно крутить во время игры без щелчков и сбоя ритма.
    pub(super) fn adopt(&mut self, state: &DroneState) {
        self.notes.clear();
        self.notes.extend(state.notes.iter().map(|n| n.as_u8()));
        self.gain = state.gain;
        self.mode = state.mode;
        self.samples_per_step = (self.sample_rate.hz() * 60.0 / state.bpm).max(1.0);
        self.pulse_duty = state.pulse_duty;
        self.arp_gate = state.arp_gate;
        self.arp_pattern = state.arp_pattern;
        self.brightness = state.brightness;
        self.timbre = state.timbre;
        // Частоты по текущему камертону: таблица на все 128 нот, индекс = MIDI.
        for (m, slot) in self.freq.iter_mut().enumerate() {
            *slot = Midi(m as f32).to_hz(state.reference_hz).0;
        }
    }

    /// Какой голос (или голоса) звучит на текущем сэмпле и с каким гейтом.
    /// Возвращает (midi, gate_open) для каждой звучащей ноты — для пульса это
    /// весь набор, для арпеджио максимум одна нота, для дрона — весь набор.
    fn sounding_target(&self, out: &mut [bool; 128]) {
        if self.notes.is_empty() {
            return;
        }
        match self.mode {
            DroneMode::Sustained => {
                for &m in &self.notes {
                    out[m as usize] = true;
                }
            }
            DroneMode::Pulse => {
                let beat_phase = (self.clock / self.samples_per_step as f64).fract() as f32;
                if beat_phase < self.pulse_duty {
                    for &m in &self.notes {
                        out[m as usize] = true;
                    }
                }
            }
            DroneMode::Arp => {
                let step_len = self.samples_per_step as f64;
                let beat_phase = (self.clock / step_len).fract() as f32;
                if beat_phase < self.arp_gate {
                    let step = (self.clock / step_len).floor() as i64;
                    let n = self.notes.len() as i64;
                    let pos = match self.arp_pattern {
                        ArpPattern::Up => step.rem_euclid(n),
                        ArpPattern::Down => n - 1 - step.rem_euclid(n),
                        ArpPattern::UpDown => {
                            // Пинг-понг: период 2*(n-1), вершина и низ не дублируются.
                            if n <= 1 {
                                0
                            } else {
                                let period = 2 * (n - 1);
                                let k = step.rem_euclid(period);
                                if k < n { k } else { period - k }
                            }
                        }
                    };
                    out[self.notes[pos as usize] as usize] = true;
                }
            }
        }
    }

    pub(super) fn next_sample(&mut self) -> f32 {
        let mut target = [false; 128];
        self.sounding_target(&mut target);

        // Глобальный LFO вибрато: считаем дёшево всегда, применяем лишь к смычку.
        self.lfo_phase = (self.lfo_phase + DRONE_VIBRATO_HZ / self.sample_rate.hz()).fract();
        let pitch_mod = if matches!(self.timbre, Timbre::Violin) {
            1.0 + DRONE_VIBRATO_DEPTH * (std::f32::consts::TAU * self.lfo_phase).sin()
        } else {
            1.0
        };
        // EPiano — единственный перкуссивный тембр: гейт умножается на затухающую
        // огибающую удара, перезапуск на фронте включения ноты.
        let percussive = matches!(self.timbre, Timbre::EPiano);

        let mut mix = 0.0_f32;
        // Нормировка тембра: яркость добавляет обертоны, держим пик ~1.
        let timbre_norm = 1.0 / (1.0 + 0.6 * self.brightness);
        for (m, &want_on) in target.iter().enumerate() {
            // Удар: на фронте включения (off→on) перезапускаем огибающую.
            if percussive && want_on && !self.was_on[m] {
                self.env[m] = 1.0;
            }
            self.was_on[m] = want_on;

            let want = if want_on { 1.0 } else { 0.0 };
            let mut a = self.amp[m];
            // Пропускаем полностью молчащий голос (и гейт, и хвост огибающей).
            if a == 0.0 && want == 0.0 && self.env[m] == 0.0 {
                continue;
            }
            a += (want - a) * self.ramp_k;
            if (a - want).abs() < 1e-4 {
                a = want;
            }
            self.amp[m] = a;
            if a <= 1e-4 {
                self.amp[m] = 0.0;
            }

            // Итоговая громкость голоса: гейт (атака/спад) × огибающая удара.
            let level = if percussive { a * self.env[m] } else { a };
            if level > 1e-4 {
                let ph = self.phase[m];
                mix += level * timbre_voice(self.timbre, ph, self.brightness) * timbre_norm;
                let next = ph + (self.freq[m] * pitch_mod) / self.sample_rate.hz();
                self.phase[m] = next.fract();
            }

            // Затухание удара продолжается, пока нота держится (зажатая клавиша
            // уходит в тишину); ниже порога обнуляем, чтобы голос выпал из цикла.
            if percussive {
                self.env[m] *= self.pluck_decay;
                if self.env[m] < 1e-5 {
                    self.env[m] = 0.0;
                }
            }
        }

        self.clock += 1.0;
        // tanh — мягкий лимитер: плотный аккорд не клиппует жёстко.
        (mix * DRONE_OUTPUT_GAIN * self.gain).tanh()
    }
}

/// Сэмпл одного голоса по тембру. `ph` — фаза в циклах (0..1), `brightness`
/// (0..1) управляет яркостью верхних гармоник. Каждый тембр нормирован к пику
/// ≈1, чтобы громкость не прыгала при переключении. Гармоник ≤12 — на самой
/// высокой дрон-ноте (~1 кГц) даже 12-я гармоника ниже Найквиста, без алиасинга.
/// Перкуссивная огибающая и вибрато живут в `next_sample`, не здесь.
fn timbre_voice(timbre: Timbre, ph: f32, brightness: f32) -> f32 {
    use std::f32::consts::TAU;
    let tau = TAU * ph;
    match timbre {
        // Чистый тон с лёгкими обертонами — прежний дефолтный голос дрона.
        Timbre::Sine => tau.sin() + brightness * (0.4 * (2.0 * tau).sin() + 0.2 * (3.0 * tau).sin()),
        // Смычковая струна: пилообразный спектр (гармоники ~1/k), brightness
        // открывает верх через спектральный наклон `tilt`.
        Timbre::Violin => {
            let tilt = 0.45 + 0.5 * brightness; // 0.45..0.95
            let mut s = 0.0;
            let mut w = 1.0;
            for k in 1..=10 {
                let kf = k as f32;
                s += (w / kf) * (kf * tau).sin();
                w *= tilt;
            }
            s * 0.55
        }
        // Орган/драубары: фундамент + октавы, тёплый и стабильный, без затухания.
        Timbre::Organ => {
            let up = 0.4 + 0.6 * brightness;
            let base = tau.sin()
                + 0.5 * (2.0 * tau).sin()
                + up * (0.6 * (3.0 * tau).sin() + 0.4 * (4.0 * tau).sin());
            base * 0.5
        }
        // Субтрактивный синт: яркая пила со «срезом» (фильтром) по brightness.
        Timbre::Synth => {
            let tilt = 0.3 + 0.65 * brightness; // 0.3..0.95
            let mut s = 0.0;
            let mut w = 1.0;
            for k in 1..=12 {
                let kf = k as f32;
                s += (w / kf) * (kf * tau).sin();
                w *= tilt;
            }
            s * 0.5
        }
        // Электропиано/удар: немного гармоник с «колокольной» 2-й; перкуссивная
        // огибающая (затухание) добавляется в next_sample через env.
        Timbre::EPiano => {
            let bell = 0.3 + 0.5 * brightness;
            (tau.sin()
                + bell * (2.0 * tau).sin()
                + 0.4 * bell * (3.0 * tau).sin()
                + 0.2 * bell * (4.0 * tau).sin())
                * 0.6
        }
    }
}
