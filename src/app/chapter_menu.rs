//! The stats bar across the top of the reading screen, and the chapter menu
//! that drops down from it.

use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, ScrollArea, Sense, Stroke, StrokeKind,
    UiBuilder, pos2, text::LayoutJob, text::TextWrapping, vec2,
};

use super::{App, finish_estimate, format_thousands, ui_font};
use crate::progress;

/// Height of the stats bar, including its progress line.
pub const BAR_HEIGHT: f32 = 56.0;
/// Width of the chapter menu (narrower if the window is).
const MENU_WIDTH: f32 = 480.0;
/// Seconds the menu takes to drop open or fold away.
const OPEN_TIME: f32 = 0.2;
/// Row heights: real chapters, and front matter (under `SLIM_WORDS` words).
const ROW_HEIGHT: f32 = 50.0;
const SLIM_HEIGHT: f32 = 24.0;
const ROW_GAP: f32 = 2.0;
const SLIM_WORDS: usize = 30;
/// How quickly the highlight glides to the row under the mouse.
const GLIDE_TIME: f32 = 0.05;

/// The color for chapter `index`, used for the little bar on its row. The
/// hue moves 28 degrees a chapter, so neighbors are related.
fn chapter_color(index: usize) -> Color32 {
    hsl((205.0 + 28.0 * index as f32) % 360.0, 0.6, 0.55)
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> Color32 {
    let a = saturation * lightness.min(1.0 - lightness);
    let channel = |n: f32| {
        let k = (n + hue / 30.0) % 12.0;
        let value = lightness - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (value * 255.0).round() as u8
    };
    Color32::from_rgb(channel(0.0), channel(8.0), channel(4.0))
}

impl App {
    /// "72 wpm    Chapter 5 · 38%    1 h 20 min left  ⚙" with the chapter's
    /// progress line under it. The chapter opens the chapter menu; the gear
    /// opens Settings.
    pub(super) fn stats_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let theme = self.settings.theme;
        let painter = ui.painter().clone();
        let y = rect.top() + 20.0;
        let left = rect.left() + 16.0;
        let right = rect.right() - 16.0;

        // The gear, always there.
        let gear = Rect::from_center_size(pos2(right - 12.0, y), vec2(30.0, 30.0));
        let gear_response = ui.interact(gear, ui.id().with("settings gear"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let gear_color = if gear_response.hovered() { theme.text } else { theme.upcoming() };
        painter.text(gear.center(), Align2::CENTER_CENTER, "⚙", FontId::proportional(21.0), gear_color);
        if gear_response.clicked() {
            self.open_settings(ui.ctx());
            return;
        }
        if self.book.is_none() {
            return;
        }

        let wpm = self.word_times.len();
        let (chapter_done, book_done) = self.fraction_done();
        painter.text(pos2(left, y), Align2::LEFT_CENTER, format!("{wpm} wpm"), FontId::proportional(17.0), theme.text);
        // When you'll finish, at the pace you've kept since first opening the book.
        let finish = finish_estimate(self.book_started, progress::now(), book_done);
        painter.text(pos2(gear.left() - 10.0, y), Align2::RIGHT_CENTER, finish, FontId::proportional(17.0), theme.upcoming());

        // The chapter, which opens the menu.
        let label = format!("Chapter {} · {:.0}%", self.chapter + 1, chapter_done * 100.0);
        let galley = painter.layout_no_wrap(label, FontId::proportional(17.0), theme.text);
        let label_rect = Rect::from_center_size(pos2(rect.center().x, y), galley.size()).expand2(vec2(10.0, 5.0));
        let response = ui
            .interact(label_rect, ui.id().with("chapter label"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() || self.menu_open {
            painter.rect_filled(label_rect, 6.0, theme.shade(0.08));
        }
        painter.galley(label_rect.center() - galley.size() / 2.0, galley, theme.text);
        self.menu_label = label_rect;
        if response.clicked() {
            self.toggle_chapter_menu();
        }

        // The chapter's progress line.
        let line = Rect::from_x_y_ranges(left..=right, rect.top() + 42.0..=rect.top() + 45.0);
        painter.rect_filled(line, 1.5, theme.shade(0.12));
        let filled = Rect::from_min_size(line.min, vec2(line.width() * chapter_done.clamp(0.0, 1.0), line.height()));
        painter.rect_filled(filled, 1.5, theme.text);
    }

    pub(super) fn toggle_chapter_menu(&mut self) {
        self.menu_open = !self.menu_open;
        if self.menu_open {
            self.menu_selected = self.chapter;
            self.menu_opened = Instant::now();
            self.glide_y = None;
        }
    }

    /// Moves the keyboard highlight up (-1) or down (+1).
    pub(super) fn move_menu_selection(&mut self, step: isize) {
        let last = self.chapter_chars.len().saturating_sub(1);
        self.menu_selected = self.menu_selected.saturating_add_signed(step).min(last);
        self.menu_keyed = true;
    }

    /// Goes to the start of chapter `index` and closes the menu. Picking the
    /// chapter you're in only closes it, so you can't lose your place.
    pub(super) fn jump_to_chapter(&mut self, index: usize) {
        self.menu_open = false;
        if index == self.chapter {
            return;
        }
        self.go_to_chapter(index);
        self.chapter_card = None;
        self.line_shown = None;
        self.cursor_x = None;
        self.trail_tail = None;
    }

    /// The chapter menu, dropping down under the stats bar.
    pub(super) fn chapter_menu(&mut self, ui: &mut egui::Ui) {
        let count = self.chapter_chars.len();
        if self.book.is_none() || count == 0 {
            return;
        }
        let dt = self.menu_frame.elapsed().as_secs_f32().min(1.0 / 30.0);
        self.menu_frame = Instant::now();
        let target = if self.menu_open { 1.0 } else { 0.0 };
        let step = dt / OPEN_TIME;
        self.menu_t = if self.menu_t < target { (self.menu_t + step).min(1.0) } else { (self.menu_t - step).max(0.0) };
        if self.menu_t != target {
            ui.ctx().request_repaint();
        }
        if self.menu_t == 0.0 {
            return;
        }
        let ease = 1.0 - (1.0 - self.menu_t).powi(3);

        let rect = ui.max_rect();
        let theme = self.settings.theme;
        let rows_height: f32 = (0..count).map(|i| self.row_height(i) + ROW_GAP).sum();
        let width = MENU_WIDTH.min(rect.width() - 32.0);
        let height = (rows_height + 60.0).min((rect.height() - BAR_HEIGHT) * 0.75);
        let top = rect.top() + BAR_HEIGHT - 4.0 - (1.0 - ease) * 12.0;
        let menu = Rect::from_center_size(pos2(rect.center().x, top + height / 2.0), vec2(width, height));

        // Its own layer, above the book text.
        let layer = LayerId::new(Order::Middle, Id::new("chapter menu"));
        let ui = &mut ui.new_child(UiBuilder::new().layer_id(layer).max_rect(menu));
        ui.set_opacity(ease);
        ui.painter().rect_filled(menu, 10.0, theme.background);
        ui.painter().rect_stroke(menu, 10.0, Stroke::new(1.0, theme.shade(0.18)), StrokeKind::Inside);

        // A click anywhere else closes it (the chapter label toggles it itself).
        let pressed_outside = ui.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !menu.contains(p) && !self.menu_label.contains(p))
        });
        if self.menu_open && pressed_outside {
            self.menu_open = false;
        }

        let (_, book_done) = self.fraction_done();
        let heading = Rect::from_min_size(menu.min + vec2(16.0, 12.0), vec2(width - 32.0, 24.0));
        ui.painter().text(heading.left_center(), Align2::LEFT_CENTER, "Chapters", ui_font(17.0, true, self.serif), theme.text);
        ui.painter().text(
            heading.right_center(),
            Align2::RIGHT_CENTER,
            format!("{:.1}% of the book", book_done * 100.0),
            FontId::proportional(12.5),
            theme.upcoming(),
        );
        let list_area = Rect::from_min_max(menu.min + vec2(8.0, 44.0), menu.max - vec2(8.0, 8.0));
        self.chapter_list(ui, list_area);
    }

    fn row_height(&self, i: usize) -> f32 {
        if self.chapter_words[i] < SLIM_WORDS { SLIM_HEIGHT } else { ROW_HEIGHT }
    }

    fn chapter_list(&mut self, ui: &mut egui::Ui, area: Rect) {
        let theme = self.settings.theme;
        let count = self.chapter_chars.len();
        let mut list_ui = ui.new_child(UiBuilder::new().max_rect(area));
        list_ui.set_clip_rect(area);

        let opened = self.menu_opened.elapsed().as_secs_f32();
        let keyed = std::mem::take(&mut self.menu_keyed);

        // The highlight glides toward the selected row: `y` is its distance
        // from the top of the list.
        let selected = self.menu_selected.min(count - 1);
        let target_y: f32 = (0..selected).map(|i| self.row_height(i) + ROW_GAP).sum();
        let dt = self.glide_frame.elapsed().as_secs_f32().min(1.0 / 60.0);
        self.glide_frame = Instant::now();
        let y = match self.glide_y {
            Some(y) if (target_y - y).abs() > 0.5 => y + (target_y - y) * (1.0 - (-dt / GLIDE_TIME).exp()),
            _ => target_y,
        };
        self.glide_y = Some(y);
        if y != target_y {
            ui.ctx().request_repaint();
        }

        let mut hovered_row = None;
        let mut clicked_row = None;
        let this = &*self;
        // Scrolls only when you scroll it; Up/Down scroll just enough to keep
        // the highlight in view.
        ScrollArea::vertical().id_salt("chapter list").auto_shrink(false).show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            let width = ui.available_width();
            let content_top = ui.cursor().top();
            // Drawn before the rows, so it's behind them.
            let glide = Rect::from_min_size(pos2(ui.cursor().left(), content_top + y), vec2(width, this.row_height(selected)));
            ui.painter().rect_filled(glide, 7.0, theme.shade(0.09));
            ui.painter().rect_stroke(glide, 7.0, Stroke::new(1.0, theme.shade(0.14)), StrokeKind::Inside);

            for i in 0..count {
                let (row, response) = ui.allocate_exact_size(vec2(width, this.row_height(i)), Sense::click());
                if response.hovered() {
                    hovered_row = Some(i);
                }
                if response.clicked() {
                    clicked_row = Some(i);
                }
                if keyed && i == selected {
                    response.scroll_to_me(None);
                }
                if !ui.is_rect_visible(row) {
                    continue;
                }
                // Rows fade and slide in one after another when the menu opens.
                let appear = ((opened - 0.03 - i.min(20) as f32 * 0.018) / 0.2).clamp(0.0, 1.0);
                if appear < 1.0 {
                    ui.ctx().request_repaint();
                }
                let shift = (1.0 - appear) * -8.0;
                this.chapter_row(ui.painter(), row.translate(vec2(shift, 0.0)), i, appear);
            }
        });
        if let Some(i) = hovered_row {
            self.menu_selected = i;
        }
        if let Some(i) = clicked_row {
            self.jump_to_chapter(i);
        }
    }

    fn chapter_row(&self, painter: &egui::Painter, row: Rect, i: usize, alpha: f32) {
        let theme = self.settings.theme;
        let fade = |c: Color32| c.gamma_multiply(alpha);
        let words = self.chapter_words[i];
        let done = if i < self.chapter { 1.0 } else if i == self.chapter { self.fraction_done().0 } else { 0.0 };
        let left = row.left() + 14.0;

        if words < SLIM_WORDS {
            let mut job = LayoutJob::simple_singleline(self.chapter_titles[i].clone(), FontId::proportional(12.5), fade(theme.upcoming()));
            job.wrap = TextWrapping::truncate_at_width(row.width() - 24.0);
            let galley = painter.layout_job(job);
            painter.galley(pos2(left, row.center().y - galley.size().y / 2.0), galley, theme.text);
            return;
        }

        // A small bar in the chapter's color.
        let stripe = Rect::from_min_size(pos2(row.left() + 4.0, row.top() + 9.0), vec2(3.0, row.height() - 18.0));
        painter.rect_filled(stripe, 1.5, fade(chapter_color(i)));

        let title_color = if i <= self.chapter { theme.text } else { theme.upcoming() };
        let right_text = if i == self.chapter { format!("{:.0}%", done * 100.0) } else { String::new() };
        let right_width = if right_text.is_empty() { 0.0 } else { 44.0 };
        let mut title = LayoutJob::simple_singleline(
            format!("{}. {}", i + 1, self.chapter_titles[i]),
            ui_font(15.0, false, self.serif),
            fade(title_color),
        );
        title.wrap = TextWrapping::truncate_at_width(row.width() - 24.0 - right_width);
        let title = painter.layout_job(title);
        painter.galley(pos2(left, row.top() + 7.0), title, theme.text);
        if !right_text.is_empty() {
            painter.text(pos2(row.right() - 10.0, row.top() + 9.0), Align2::RIGHT_TOP, right_text, FontId::proportional(12.5), fade(theme.upcoming()));
        }

        let detail = if i == self.chapter {
            let left = (words as f32 * (1.0 - done)).round() as u64;
            format!("{} words left", format_thousands(left))
        } else if i < self.chapter {
            match self.chapter_records.get(&i) {
                Some((wpm, accuracy)) => format!("{wpm} wpm · {} accuracy", super::format_accuracy(*accuracy)),
                None => "Finished".to_string(),
            }
        } else {
            format!("{} words", format_thousands(words as u64))
        };
        painter.text(pos2(left, row.top() + 29.0), Align2::LEFT_TOP, detail, FontId::proportional(12.0), fade(theme.upcoming()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_matches_known_colors() {
        assert_eq!(hsl(0.0, 1.0, 0.5), Color32::from_rgb(255, 0, 0));
        assert_eq!(hsl(120.0, 1.0, 0.5), Color32::from_rgb(0, 255, 0));
        assert_eq!(hsl(240.0, 1.0, 0.5), Color32::from_rgb(0, 0, 255));
        assert_eq!(hsl(0.0, 0.0, 0.5), Color32::from_rgb(128, 128, 128));
    }
}
