//! Каркас карточки Scale Finder: тик решалки, вызов ранжирования и раскладка
//! двух колонок (улитка + кольцо квинт слева, вердикты методов справа).
//!
//! ИЗОЛЯЦИЯ: весь расчёт внутри `draw_scale_finder_card`. egui_tiles зовёт
//! `pane_ui` только для видимой вкладки — пока панель закрыта, ничего не считается.
//! Состояния в `App` детектор не держит.

use eframe::egui::{
    self,
    CornerRadius,
    Frame,
    Margin,
    RichText,
    Sense,
    Stroke,
    Ui,
    pos2,
    vec2,
};

use super::super::{
    App,
    ScaleKind,
    pill,
    pill_muted,
};
use super::ranking::{
    Ranking,
    rank,
};
use super::{
    panel,
    wheel,
};
use crate::core_types::note::AccidentalStyle;
use crate::core_types::pitch::PCNote;
use crate::ui::tokens::color;

impl App {
    pub(in crate::app) fn draw_scale_finder_card(&mut self, ui: &mut Ui) {
        // Эта панель — потребитель резонаторного банка: пока она видима, держим
        // его «нужным». Закрылась → запросы прекратились → банк паркуется.
        self.audio.request_resonator();

        let now = web_time::Instant::now();
        let reading = self.audio.reading();
        let settings = self.audio.analysis_settings();

        // Тик решалки — только пока панель видима (этот метод зовётся лишь для
        // активной вкладки): закрыта → не тикается, окно стынет, ничего не считается.
        if let Some(reading) = &reading {
            self.scale_solver.tick(now, reading, &settings);
        }
        let (chroma_frames, bass_frames) = self.scale_solver.window(now, self.scale_finder.window_seconds);
        let ranking = rank(&chroma_frames, &bass_frames, self.scale_finder);
        let captured_secs = self.scale_solver.captured_secs(now);

        let selected_root_pc = PCNote::from_natural(self.root_note).0 as usize;
        let selected_kind = self.scale_kind;

        Frame::new()
            .fill(color::PANEL_FILL)
            .corner_radius(CornerRadius::same(22))
            .stroke(Stroke::new(1.0_f32, color::CARD_STROKE))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Scale finder")
                                .size(20.0)
                                .color(color::TEXT_HEADING),
                        );
                        ui.label(
                            RichText::new("4 methods on the snail: notes · tonal · root · spiral")
                                .color(color::TEXT_HINT),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match ranking.as_ref().and_then(|r| r.candidates.first()) {
                            Some(top) => {
                                pill(
                                    ui,
                                    &format!(
                                        "{:.0}%  {}",
                                        top.probability * 100.0,
                                        top.label(settings.accidental)
                                    ),
                                )
                            }
                            None => pill_muted(ui, "waiting for input"),
                        }
                    });
                });

                ui.add_space(10.0);
                self.draw_scale_finder_controls(ui, captured_secs);
                ui.add_space(10.0);
                draw_scale_finder_body(
                    ui,
                    ranking.as_ref(),
                    selected_root_pc,
                    selected_kind,
                    settings.accidental,
                );
            });
    }
}

fn draw_scale_finder_body(
    ui: &mut Ui,
    ranking: Option<&Ranking>,
    selected_root_pc: usize,
    selected_kind: ScaleKind,
    style: AccidentalStyle,
) {
    use eframe::egui::{
        FontId,
        Rect,
    };

    let available = ui.available_size_before_wrap();
    let desired = vec2(available.x, available.y.max(420.0));
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 18.0, color::PLOT_BG);
    painter.rect_stroke(
        rect,
        18.0,
        Stroke::new(1.0_f32, color::PLOT_STROKE),
        egui::StrokeKind::Inside,
    );

    let Some(ranking) = ranking else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Play a sustained phrase — the snail will rank key, scale and mode",
            FontId::proportional(13.0),
            color::TEXT_HINT,
        );
        return;
    };

    let pad = 16.0;
    let inner = rect.shrink(pad);
    let wheel_w = inner.width() * 0.42;
    let wheel_rect = Rect::from_min_size(inner.min, vec2(wheel_w, inner.height()));
    let list_rect = Rect::from_min_max(pos2(wheel_rect.right() + pad, inner.top()), inner.max);

    // Левая колонка делится на хроматическую улитку (chroma по полутонам) и кольцо
    // КВИНТ (метод D): на нём центр тяжести показан стрелкой к тональному центру.
    let snail_h = wheel_rect.height() * 0.58;
    let snail_rect = Rect::from_min_size(wheel_rect.min, vec2(wheel_rect.width(), snail_h));
    let fifths_rect = Rect::from_min_max(pos2(wheel_rect.left(), snail_rect.bottom() + 6.0), wheel_rect.max);

    wheel::draw_chroma_wheel(&painter, snail_rect, ranking, style);
    wheel::draw_fifths_ring(&painter, fifths_rect, ranking, style);
    panel::draw_method_panel(
        &painter,
        list_rect,
        ranking,
        selected_root_pc,
        selected_kind,
        style,
    );
}
