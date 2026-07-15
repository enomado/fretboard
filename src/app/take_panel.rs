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
use crate::audio::RecorderStatus;
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
                ui.label(RichText::new("This session").color(color::TEXT_CAPTION).strong());
                ui.add_space(6.0);
                self.draw_session_takes(ui);
            });
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
            // остановить можно всегда.
            let armed = recording || !self.take_name.trim().is_empty();
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

    /// Что записано с запуска приложения.
    ///
    /// Только эта сессия, и это не лень: отчёты уже лежат в памяти, поэтому
    /// список ничего не читает с диска. Показать так же длину и приговор для
    /// СТАРЫХ дублей значило бы декодировать их WAV'ы — то есть завести
    /// приложению входной путь для аудио, которого у него нет и который никто не
    /// заказывал (см. `audio::native::imp::recorder`).
    fn draw_session_takes(&self, ui: &mut Ui) {
        if self.takes.is_empty() {
            ui.label(
                RichText::new("Nothing recorded yet — takes land in testdata/")
                    .color(color::TEXT_MUTED)
                    .size(12.0),
            );
            return;
        }

        for report in &self.takes {
            ui.horizontal(|ui| {
                // Приговор первым и значком: в списке из десятка дублей глаз ищет
                // не имя, а «какие годятся».
                let (mark, tint) = if report.is_evidence() {
                    ("✓", color::STATUS_LISTENING)
                } else {
                    ("✗", color::STATUS_ERROR)
                };
                ui.label(RichText::new(mark).color(tint).monospace());
                ui.label(
                    RichText::new(take_label(&report.path))
                        .color(color::TEXT_VALUE)
                        .monospace()
                        .size(12.0),
                );
                ui.label(
                    RichText::new(format!("{:.1} s · {} Hz", report.seconds(), report.sample_rate))
                        .color(color::TEXT_HINT)
                        .monospace()
                        .size(12.0),
                );
                if !report.is_evidence() {
                    ui.label(
                        RichText::new(format!("{} samples dropped — not evidence", report.dropped))
                            .color(color::STATUS_ERROR)
                            .size(12.0),
                    );
                }
            });
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(take_file_name(name))
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
