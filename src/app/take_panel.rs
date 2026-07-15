//! Take Recorder panel — where evidence is made.
//!
//! Its own panel rather than a row in Controls, and the split is not cosmetic:
//! Controls configures the *input path* (source, device, gain, monitor), while
//! this one *produces takes*. Different questions, different rooms.
//!
//! It is also the room the annotator will move into — Ф2 of
//! `memory/kickstart_recording_and_annotation.md`: a scrollable, zoomable pitch
//! roll to mark note boundaries on. That is a workbench, not a row.

use std::path::{
    Path,
    PathBuf,
};

use eframe::egui::{
    self,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Stroke,
    Ui,
};

use super::App;
use crate::audio::{
    RecorderStatus,
    ReplayStatus,
};
use crate::ui::segmented::{
    RowCaption,
    SegmentedButton,
};
use crate::ui::tokens::color;

impl App {
    pub(super) fn draw_take_recorder_card(&mut self, ui: &mut Ui) {
        let frame_width = ui.available_width();

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(18))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_min_width(frame_width - 32.0);
                self.draw_take_controls(ui);
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);
                ui.label(RichText::new("Corpus").color(color::TEXT_CAPTION).strong());
                ui.add_space(6.0);
                self.draw_corpus(ui);
            });
    }

    /// Перечитать опись `testdata/`.
    ///
    /// Зовётся при старте и после каждого законченного дубля — то есть ровно тогда,
    /// когда содержимое папки могло измениться нашими руками. Не покадрово: это
    /// открытие файлов, а корпус не меняется сам по себе.
    pub(super) fn refresh_corpus(&mut self) {
        self.corpus = self.audio.list_takes(&take_dir());
    }

    /// Имя, кнопка, приговор по текущему/последнему дублю.
    fn draw_take_controls(&mut self, ui: &mut Ui) {
        let status = self.audio.recorder_status();

        // Браузерная сборка писать некуда — и говорит об этом, вместо того чтобы
        // показать кнопку, которая делает вид, что записала.
        if status == RecorderStatus::Unsupported {
            ui.label(
                RichText::new(
                    "Take recording needs the desktop build — the browser engine has no filesystem",
                )
                .color(color::TEXT_HINT)
                .size(12.0),
            );
            return;
        }

        let recording = matches!(status, RecorderStatus::Recording { .. });
        // Пока реплей держит capture, микрофона на входе нет: писать нечего. Дубль
        // — это то, что сыграла скрипка, а не то, что мы сами же и проигрываем (см.
        // `audio::native::imp::replay`); кнопка это признаёт, а не делает вид.
        let replaying = !matches!(
            self.audio.replay_status(),
            ReplayStatus::Idle | ReplayStatus::Unsupported
        );

        ui.horizontal(|ui| {
            ui.add(RowCaption::new("Take"));

            // Посреди дубля имя менять нечему: писатель уже держит файл открытым,
            // и переименование не переоткрывает его, а начинает НОВЫЙ дубль.
            ui.add_enabled(
                !recording,
                egui::TextEdit::singleline(&mut self.take_name)
                    .desired_width(240.0)
                    .hint_text("g_flageolet_f5"),
            );

            let (label, fill, stroke) = if recording {
                ("Stop", color::STOP_FILL, color::STOP_STROKE)
            } else {
                ("Record", color::PLAY_FILL, color::PLAY_STROKE)
            };
            // Без имени писать некуда — но только для старта: начатый дубль
            // остановить можно всегда. При реплее старт запрещён (см. `replaying`).
            let armed = recording || (!replaying && !self.take_name.trim().is_empty());
            let button = SegmentedButton::colored(label, fill, stroke).min_width(88.0);
            if ui.add_enabled(armed, button).clicked() {
                if recording {
                    self.audio.stop_take();
                } else {
                    self.audio.start_take(take_path(&self.take_name));
                }
            }
        });

        ui.add_space(6.0);
        draw_take_status(ui, &status);
    }

    /// Что лежит в `testdata/` и что из этого можно проиграть.
    ///
    /// Приговор «улика / не улика» стоит **только** у дублей этой сессии: он про
    /// запись (сколько сэмплов потерялось), а не про файл, и с диска его не
    /// прочитать — WAV с дырой валиден побайтово. Врать нулём вместо «не знаю» тут
    /// нельзя: ровно от этой лжи и заведён счётчик (см. `TakeOnDisk`).
    fn draw_corpus(&mut self, ui: &mut Ui) {
        if self.corpus.is_empty() {
            ui.label(
                RichText::new("No takes in testdata/ yet — record one above")
                    .color(color::TEXT_MUTED)
                    .size(12.0),
            );
            return;
        }

        let replay = self.audio.replay_status();
        let playing_now = match &replay {
            ReplayStatus::Playing { path, .. } => Some(path.clone()),
            _ => None,
        };
        // Реплей держит capture целиком, так что играть два дубля разом нельзя —
        // и второй Play не должен выглядеть доступным.
        let replay_busy = playing_now.is_some();

        for take in &self.corpus {
            let is_playing = playing_now.as_deref() == Some(take.path.as_path());
            ui.horizontal(|ui| {
                let (label, fill, stroke) = if is_playing {
                    ("■", color::STOP_FILL, color::STOP_STROKE)
                } else {
                    ("▶", color::PLAY_FILL, color::PLAY_STROKE)
                };
                let button = SegmentedButton::colored(label, fill, stroke).min_width(32.0);
                if ui.add_enabled(is_playing || !replay_busy, button).clicked() {
                    if is_playing {
                        self.audio.stop_replay();
                    } else {
                        self.audio.start_replay(take.path.clone());
                    }
                }

                // Приговор — только для дублей ЭТОЙ сессии; про остальные молчим,
                // потому что не знаем.
                let verdict = self
                    .takes
                    .iter()
                    .find(|report| report.path == take.path)
                    .map(|report| report.is_evidence());
                let (mark, tint) = match verdict {
                    Some(true) => ("✓", color::STATUS_LISTENING),
                    Some(false) => ("✗", color::STATUS_ERROR),
                    None => (" ", color::TEXT_MUTED),
                };
                ui.label(RichText::new(mark).color(tint).monospace());

                ui.label(
                    RichText::new(take_label(&take.path))
                        .color(if is_playing {
                            color::STATUS_LISTENING
                        } else {
                            color::TEXT_VALUE
                        })
                        .monospace()
                        .size(12.0),
                );
                ui.label(
                    RichText::new(format!("{:.1} s · {} Hz", take.seconds(), take.sample_rate))
                        .color(color::TEXT_HINT)
                        .monospace()
                        .size(12.0),
                );
                if verdict == Some(false) {
                    ui.label(
                        RichText::new("samples dropped — not evidence")
                            .color(color::STATUS_ERROR)
                            .size(12.0),
                    );
                }
            });
        }

        ui.add_space(8.0);
        draw_replay_status(ui, &replay);

        // Дубль доиграл, но capture всё ещё реплейный (линия заморожена — её и
        // размечают). Вернуть живой вход — отдельный осознанный клик.
        if matches!(replay, ReplayStatus::Finished { .. }) {
            ui.add_space(6.0);
            let button = SegmentedButton::colored("Back to live input", color::STOP_FILL, color::STOP_STROKE)
                .min_width(150.0);
            if ui.add(button).clicked() {
                self.audio.stop_replay();
            }
        }
    }

    /// Подобрать законченный дубль в список сессии.
    ///
    /// Вызывается из `eframe::App::ui` каждый кадр, а **не** из отрисовки панели.
    /// egui_tiles не рисует невыбранные вкладки, так что дубль, законченный при
    /// закрытой панели, в список бы не попал — а запись идёт в аудио-треде и на
    /// UI не смотрит вообще. Список не должен зависеть от того, куда смотрит юзер.
    ///
    /// Рекордер держит `Finished` до старта следующего дубля, так что покадровый
    /// опрос его не пропустит; сверка с последним элементом отсеивает повторные
    /// прочтения того же отчёта.
    pub(super) fn harvest_finished_take(&mut self) {
        let RecorderStatus::Finished(report) = self.audio.recorder_status() else {
            return;
        };
        if self.takes.last() == Some(&report) {
            return;
        }
        self.takes.push(report);
        // Файл только что появился — опись устарела ровно сейчас, и это
        // единственный момент, когда её надо перечитать.
        self.refresh_corpus();
    }
}

/// Куда ложится дубль.
///
/// `testdata/` в дереве ИСХОДНИКОВ — туда же, где живёт корпус, в который дубль
/// и вступает, и который коммитится вместе с кодом. Отсюда `CARGO_MANIFEST_DIR`,
/// а не рабочая директория: дубль не должен зависеть от того, откуда запустили
/// приложение. Запись дублей — занятие разработческое, так что путь времени
/// сборки здесь честнее рантаймового; на телефоне его нет, запись там честно
/// упадёт с ошибкой пути, а не запишет дубль в никуда.
fn take_path(name: &str) -> PathBuf {
    take_dir().join(take_file_name(name))
}

/// Корпус — `testdata/` в дереве ИСХОДНИКОВ. Отсюда же его и читает реплей, и
/// пишет рекордер, и грузят тесты: одна папка, один смысл. См. [`take_path`].
fn take_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// Имя из поля ввода → имя файла.
///
/// Всё, кроме ASCII-букв/цифр/`_`/`-`, схлопывается в `_`. Две причины: имена
/// корпуса живут в командных строках и путях (`g_open_slow_strokes.wav`), и так
/// `/` или `..`, набранные в поле, не уводят запись за пределы `testdata/`.
fn take_file_name(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{slug}.wav")
}

/// Приговор по дублю.
///
/// Главное здесь — `dropped`. Дубль с дырой выглядит нормально, звучит нормально
/// и открывается любым плеером; единственное, что отличает его от улики, — это
/// число. Поэтому оно на экране, а не в логе, и поэтому оно видно ещё во время
/// записи: узнать, что дубль испорчен, надо пока его не поздно переиграть.
fn draw_take_status(ui: &mut Ui, status: &RecorderStatus) {
    let (text, tint) = match status {
        // Отфильтрован вызывающим (`draw_take_controls`).
        RecorderStatus::Unsupported => return,
        RecorderStatus::Idle => {
            (
                "Takes land in testdata/ at the device's own rate, tapped before the gain".to_owned(),
                color::TEXT_HINT,
            )
        }
        RecorderStatus::Recording {
            path,
            seconds,
            dropped,
        } => {
            if *dropped > 0 {
                (
                    format!("● {seconds:.1} s — {dropped} samples dropped, this take is not evidence"),
                    color::STATUS_ERROR,
                )
            } else {
                (
                    format!("● {seconds:.1} s → {}", take_label(path)),
                    color::STATUS_LISTENING,
                )
            }
        }
        RecorderStatus::Finished(report) => {
            if report.is_evidence() {
                (
                    format!(
                        "Saved {} — {:.1} s at {} Hz",
                        take_label(&report.path),
                        report.seconds(),
                        report.sample_rate
                    ),
                    color::STATUS_LISTENING,
                )
            } else {
                (
                    format!(
                        "{} has a hole: {} samples dropped. Not evidence — record it again",
                        take_label(&report.path),
                        report.dropped
                    ),
                    color::STATUS_ERROR,
                )
            }
        }
        RecorderStatus::Failed(message) => (message.clone(), color::STATUS_ERROR),
    };

    ui.label(RichText::new(text).color(tint).size(12.0));
}

/// Что делает реплей.
///
/// Главное здесь — сказать, что линия на pitch roll СЕЙЧАС не про микрофон, а про
/// дубль. Без этой строки картинка врёт: она выглядит ровно как живая игра, потому
/// что её рисует тот же код (и это ровно то, чего мы добивались).
fn draw_replay_status(ui: &mut Ui, status: &ReplayStatus) {
    let (text, tint) = match status {
        // На вебе реплея нет — как и рекордера, и по той же причине: нет диска.
        // Молчим, а не показываем мёртвую строку: список дублей там всё равно пуст.
        ReplayStatus::Unsupported => return,
        ReplayStatus::Idle => {
            (
                "Play a take to see the line the detector draws for it — live path, real time".to_owned(),
                color::TEXT_HINT,
            )
        }
        ReplayStatus::Playing {
            path,
            seconds,
            total_seconds,
        } => {
            (
                format!(
                    "▶ {} — {seconds:.1} / {total_seconds:.1} s · live input is off while replaying",
                    take_label(path)
                ),
                color::STATUS_LISTENING,
            )
        }
        // Стоим на конце дубля намеренно: линия, которую юзер смотрит, — это то,
        // что он сейчас размечает, и живой вход писал бы поверх неё.
        ReplayStatus::Finished { path } => {
            (
                format!(
                    "{} played to the end — the line is frozen. Stop to hand the input back",
                    take_label(path)
                ),
                color::TEXT_VALUE,
            )
        }
        ReplayStatus::Failed(message) => (message.clone(), color::STATUS_ERROR),
    };

    ui.label(RichText::new(text).color(tint).size(12.0));
}

/// Дубль на экране — это имя файла: путь до `testdata/` одинаков у всех и только
/// съедает строку. `unwrap` держится [`take_path`], которая всегда строит путь с
/// именем файла на конце.
fn take_label(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Имя из поля ввода попадает в путь, поэтому `/` и `..` в нём — это выход за
    /// пределы `testdata/`. Поле маленькое и безобидное на вид, так что пусть
    /// проверка стоит рядом.
    #[test]
    fn a_take_name_cannot_escape_testdata() {
        // `..` + `/` + `..` + `/` = шесть символов, шесть подчёркиваний.
        assert_eq!(take_file_name("../../etc/passwd"), "______etc_passwd.wav");
        assert_eq!(take_file_name("g/f5"), "g_f5.wav");
        assert_eq!(take_file_name("  g_flageolet_f5  "), "g_flageolet_f5.wav");

        let path = take_path("../boom");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "testdata");
    }
}
