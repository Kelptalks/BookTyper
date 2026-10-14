mod chapter_menu;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Button, CentralPanel, Color32, FontData, FontDefinitions, FontFamily, FontId, Key, Layout,
    Panel, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, TextFormat, ViewportCommand, Visuals,
    Galley, color_picker, pos2, text::CCursor, text::LayoutJob, text::TextWrapping, vec2,
};

use crate::epub::{self, BlockKind, Book, Style};
use crate::library::{self, Entry};
use crate::marble::{self, WEB_PER_LETTER};
use crate::progress::{self, Position};
use crate::theme::{Background, MAX_LAYERS, MAX_STRENGTH, PRESETS, Settings, Theme, Veins};
use crate::typing::Para;

const WRONG: Color32 = Color32::from_rgb(240, 95, 95);
const WRONG_BG: Color32 = Color32::from_rgba_premultiplied(60, 20, 20, 60);

const LINE_SIZE: f32 = 34.0;
/// Text wraps at this width (or the window width, if that's smaller).
const MAX_LINE_WIDTH: f32 = 900.0;
/// Width of the home screen's book list.
const COLUMN_WIDTH: f32 = 760.0;
/// Space at the top of the reading screen for the stats bar.
const HEADER_HEIGHT: f32 = chapter_menu::BAR_HEIGHT;
/// Distance between lines, as a multiple of the row height.
const LINE_GAP: f32 = 1.6;
/// The text sits on a giant cylinder that turns toward you, and we see 10% of
/// its way around: 18 degrees above and below the line you're typing, where
/// lines have faded out completely...
const DRUM_EDGE: f32 = std::f32::consts::PI * 0.1;
/// ...which puts the edge just past the third line each way (about 5 degrees a line).
const DRUM_LINE_ANGLE: f32 = DRUM_EDGE / (LINES_ABOVE as f32 + 0.5);
/// Distance from your eye to the cylinder, in cylinder radii. Smaller makes
/// the far lines shrink more.
const DRUM_FOCAL: f32 = 2.5;
/// Finished lines shown above the one you're typing.
const LINES_ABOVE: usize = 3;
/// Upcoming lines shown below the one you're typing.
const LINES_BELOW: usize = 3;
/// Seconds the lines take to slide to a new line.
const SLIDE_TIME: f32 = 0.13;
/// How far the slide overshoots before settling back (0 = no overshoot).
/// Kept small: it overshoots by about 2% of a line.
const SLIDE_SPRING: f32 = 0.8;
/// Seconds the shine takes to cross a word typed with no mistakes, and how
/// strongly it tints the letters toward the cursor color.
const SHIMMER_TIME: f32 = 0.45;
const SHIMMER_STRENGTH: f32 = 0.5;
/// Distance between the background dots, and how fast they move compared to
/// the text (less than 1 so they look farther away).
const DOT_SPACING: f32 = 36.0;
const DOT_PARALLAX: f32 = 0.35;
/// Seconds per cursor blink (on + off).
const BLINK_PERIOD: f32 = 1.0;
/// How quickly the cursor glides to the next letter: roughly the seconds to
/// cover two thirds of the way.
const GLIDE_TIME: f32 = 0.035;
/// How far the cursor's trail lags behind it: roughly the seconds its tail
/// takes to cover two thirds of the way to the cursor...
const TRAIL_LAG: f32 = 0.12;
/// ...and how bright it is at the cursor end.
const TRAIL_ALPHA: f32 = 0.5;
/// Seconds a correct letter glows in the cursor color before settling.
const POP_TIME: f32 = 0.25;
/// Seconds a correct letter takes to hop up and land, and how high, in points.
const JUMP_TIME: f32 = 0.18;
const JUMP_HEIGHT: f32 = 5.0;
/// At the top of the hop a letter is this much bigger (0.08 = 108%)...
const HOP_GROW: f32 = 0.08;
/// ...each hop's height varies by up to this much either way (0.2 = +-20%)...
const HOP_HEIGHT_VARIES: f32 = 0.2;
/// ...and it leans up to this many radians either way (about 3 degrees),
/// staying tilted until its word is finished.
const HOP_TILT: f32 = 0.052;
/// When a word is finished its letters settle: they straighten up, dip this
/// many points together and spring back, over this many seconds.
const SETTLE_TIME: f32 = 0.28;
const SETTLE_DIP: f32 = 2.0;
/// Seconds a finished line flashes in the cursor color, with a light
/// sweeping along it.
const FLASH_TIME: f32 = 0.4;
/// Seconds a typed letter takes to fill with color, from the bottom up.
const INK_TIME: f32 = 0.15;
/// Capitals and punctuation hop this much higher.
const BIG_HOP: f32 = 1.35;
/// Speed streaks: how many particles, how many letters a second count as
/// full speed, and how quickly the streaks catch up with your typing.
const STREAK_COUNT: usize = 140;
const STREAK_FULL_SPEED: f32 = 9.0;
const STREAK_EASE: f32 = 0.6;
/// A wrong letter you backspace falls this far and fades over this long.
const DISSOLVE_TIME: f32 = 0.35;
const DISSOLVE_DROP: f32 = 10.0;
/// Seconds a wrong letter shakes, and how far, in points.
const SHAKE_TIME: f32 = 0.18;
const SHAKE_DISTANCE: f32 = 4.0;
/// Chapters with fewer words typed than this (title pages, short front
/// matter) don't get an end-of-chapter card.
const CARD_MIN_WORDS: usize = 30;
/// Seconds the card takes to fade in, and before Space can dismiss it (so a
/// Space typed in the same burst doesn't skip it).
const CARD_FADE: f32 = 0.3;
const CARD_DELAY: f32 = 0.6;
/// How fast the pretend typist in the background preview types.
const PREVIEW_LETTERS_PER_SECOND: f32 = 6.0;
/// A pause longer than this between keys doesn't count as typing time.
const IDLE_CAP: Duration = Duration::from_secs(5);
const WORD_WINDOW: Duration = Duration::from_secs(60);
/// The F3 overlay reports the worst frame over this long.
const FRAME_WINDOW: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Home,
    Reading,
    Settings,
}

pub struct App {
    screen: Screen,
    /// Books on the home screen, sorted: last read first, then started ones.
    library: Vec<Entry>,
    /// Books still arriving from the background scan.
    library_scan: Option<Receiver<Entry>>,
    /// The book highlighted on the home screen.
    selected: Option<PathBuf>,
    /// Scroll the home screen so `selected` is visible (after arrow keys).
    scroll_to_selected: bool,
    /// The book you read most recently; listed first.
    last_book: Option<PathBuf>,
    book: Option<Book>,
    /// Where the book was loaded from; progress is saved under this path.
    book_path: Option<PathBuf>,
    /// Number of characters to type in each chapter, for the book percentage.
    chapter_chars: Vec<usize>,
    /// Words in the whole book.
    book_words: usize,
    /// When the open book was first opened (Unix seconds), for the finish date.
    book_started: u64,
    /// Last position handed to `saver`.
    saved: Option<Position>,
    saver: progress::Saver,
    chapter: usize,
    paras: Vec<Para>,
    /// Index into `paras` of the paragraph being typed.
    current: usize,
    /// Typed past the last paragraph of the last chapter.
    finished: bool,
    error: Option<String>,
    /// Georgia loaded from Windows' fonts (real bold/italic), else egui's default font.
    serif: bool,
    /// The cursor stays solid while typing and only blinks once you pause.
    last_key: Instant,
    /// (chapter, paragraph, line within paragraph) the cursor was on last frame.
    line_shown: Option<(usize, usize, usize)>,
    /// Vertical offset the current slide started from, in points.
    slide_from: f32,
    slide_start: Instant,
    /// When each recent word was finished; older than `WORD_WINDOW` gets dropped.
    word_times: VecDeque<Instant>,
    /// Words typed across every book, ever (shown on the home screen).
    total_words: u64,
    /// F3 toggles the frame-time overlay.
    show_frame_times: bool,
    /// (when, our ui code in ms, eframe's whole-frame CPU time in ms) for recent frames.
    frame_times: VecDeque<(Instant, f32, f32)>,
    /// Colors in use and saved custom themes.
    settings: Settings,
    /// Changed since last written to disk.
    settings_changed: bool,
    /// The last settings write, if it may still be running.
    settings_write: Option<std::thread::JoinHandle<()>>,
    /// Where Esc on the settings screen goes back to.
    settings_back: Screen,
    /// The name box on the settings screen.
    new_theme_name: String,
    /// Where the cursor is drawn across its line (it glides toward the letter
    /// it belongs at). None snaps it straight there.
    cursor_x: Option<f32>,
    cursor_frame: Instant,
    /// Where the tail end of the cursor's trail is drawn (x along the line);
    /// it chases the drawn cursor.
    trail_tail: Option<f32>,
    trail_frame: Instant,
    /// (paragraph, character, when) for recently typed correct letters, which glow briefly.
    /// Each also has its own random hop height (multiplier) and lean (radians).
    pops: Vec<(usize, usize, Instant, f32, f32)>,
    /// Random number state for the hops.
    rng: u64,
    /// When each recent correct letter was typed (last couple of seconds), for
    /// how fast you're typing right now.
    key_times: VecDeque<Instant>,
    /// That speed, eased: 0 = stopped, 1 = `STREAK_FULL_SPEED`.
    speed: f32,
    /// Speed streak particles: x and y from -1 to 1 around the middle, and
    /// depth from 1 (far) down to 0 (passing you).
    streaks: Vec<[f32; 3]>,
    streak_frame: Instant,
    /// A tile of random specks for the paper grain background.
    grain: egui::TextureHandle,
    /// The marble background behind the text, and the one in the settings
    /// preview (which scrolls on its own, as if someone were typing).
    web: marble::Web,
    preview_web: marble::Web,
    preview_start: Instant,
    /// The marble shader could be set up (it needs wgpu).
    marble_ready: bool,
    /// (paragraph, character, the wrong letter, when) for backspaced mistakes,
    /// which fall and fade.
    ghosts: Vec<(usize, usize, char, Instant)>,
    /// (paragraph, character, when) for the last wrong letter, which shakes.
    shake: Option<(usize, usize, Instant)>,
    /// (paragraph, first character, end, when) for words just typed with no mistakes.
    shimmers: Vec<(usize, usize, usize, Instant)>,
    /// (paragraph, first character, end, when) for words just finished, settling.
    settles: Vec<(usize, usize, usize, Instant)>,
    /// The line after the one just finished, and when, for the finished line's flash.
    line_flash: Option<((usize, usize, usize), Instant)>,
    /// A wrong key was typed since the current word began.
    mistake_in_word: bool,
    /// How far the text has scrolled, in points, for the background dots.
    scroll: f32,
    /// Typing in the current chapter, for the end-of-chapter card.
    chapter_stats: ChapterStats,
    /// Shown in place of the text after finishing a chapter.
    chapter_card: Option<ChapterCard>,
    /// Per chapter of the open book: title for the sidebar, and words.
    chapter_titles: Vec<String>,
    chapter_words: Vec<usize>,
    /// (wpm, accuracy %) from each chapter's end-of-chapter card, kept on disk.
    chapter_records: HashMap<usize, (u32, f32)>,
    /// The chapter menu: wanted open, and how far open it is (0 to 1, animated).
    menu_open: bool,
    menu_t: f32,
    menu_frame: Instant,
    menu_opened: Instant,
    /// The highlighted row (follows the mouse, or Up/Down while it's open).
    menu_selected: usize,
    /// Up/Down just moved the highlight: scroll the list just enough to show it.
    menu_keyed: bool,
    /// Where the stats bar's "Chapter 5 · 38%" is (clicking it toggles the menu).
    menu_label: Rect,
    /// Where the highlight is drawn, gliding toward `menu_selected`.
    glide_y: Option<f32>,
    glide_frame: Instant,
}

#[derive(Default, Clone, Copy)]
struct ChapterStats {
    words: usize,
    /// Letters typed (not counting Space/Enter that move on, or backspaces).
    keys: usize,
    mistakes: usize,
    /// Time spent typing, with long pauses left out.
    active: Duration,
}

impl ChapterStats {
    fn wpm(&self) -> u32 {
        let minutes = self.active.as_secs_f32() / 60.0;
        if minutes > 0.0 { (self.words as f32 / minutes).round() as u32 } else { 0 }
    }

    /// Percent of letters typed right. Only 100 with no mistakes at all.
    fn accuracy(&self) -> f32 {
        if self.keys == 0 || self.mistakes == 0 {
            return 100.0;
        }
        ((self.keys - self.mistakes) as f32 / self.keys as f32 * 100.0).min(99.9)
    }
}

struct ChapterCard {
    title: String,
    stats: ChapterStats,
    shown: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let settings = Settings::load();
        apply_theme(&cc.egui_ctx, &settings.theme);
        cc.egui_ctx.set_zoom_factor(settings.zoom);
        let serif = install_fonts(&cc.egui_ctx);
        let marble_ready = cc.wgpu_render_state.as_ref().map(marble::install).is_some();
        let mut app = App {
            screen: Screen::Home,
            library: Vec::new(),
            library_scan: None,
            selected: None,
            scroll_to_selected: false,
            last_book: None,
            book: None,
            book_path: None,
            chapter_chars: Vec::new(),
            book_words: 0,
            book_started: 0,
            saved: None,
            saver: progress::Saver::new(),
            chapter: 0,
            paras: Vec::new(),
            current: 0,
            finished: false,
            error: None,
            serif,
            last_key: Instant::now(),
            line_shown: None,
            slide_from: 0.0,
            slide_start: Instant::now(),
            word_times: VecDeque::new(),
            total_words: progress::total_words(),
            show_frame_times: false,
            frame_times: VecDeque::new(),
            settings,
            settings_changed: false,
            settings_write: None,
            settings_back: Screen::Home,
            new_theme_name: String::new(),
            cursor_x: None,
            cursor_frame: Instant::now(),
            trail_tail: None,
            trail_frame: Instant::now(),
            pops: Vec::new(),
            grain: grain_texture(&cc.egui_ctx),
            web: marble::Web::new(0),
            preview_web: marble::Web::new(1),
            preview_start: Instant::now(),
            marble_ready,
            key_times: VecDeque::new(),
            speed: 0.0,
            streaks: Vec::new(),
            streak_frame: Instant::now(),
            rng: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1,
            shake: None,
            ghosts: Vec::new(),
            shimmers: Vec::new(),
            settles: Vec::new(),
            line_flash: None,
            mistake_in_word: false,
            scroll: 0.0,
            chapter_stats: ChapterStats::default(),
            chapter_card: None,
            chapter_titles: Vec::new(),
            chapter_words: Vec::new(),
            chapter_records: HashMap::new(),
            menu_open: false,
            menu_t: 0.0,
            menu_frame: Instant::now(),
            menu_opened: Instant::now(),
            menu_selected: 0,
            menu_keyed: false,
            menu_label: Rect::NOTHING,
            glide_y: None,
            glide_frame: Instant::now(),
        };
        match path {
            Some(path) => app.open(&path),
            None => app.go_home(&cc.egui_ctx),
        }
        app
    }

    /// Saves your place and shows the library, rescanning it for fresh percentages.
    fn go_home(&mut self, ctx: &egui::Context) {
        self.save_progress();
        self.screen = Screen::Home;
        self.last_book = self.book_path.clone().or_else(progress::last_book);
        self.selected = self.last_book.clone();
        self.scroll_to_selected = true;
        self.library.clear();
        let current = self.book_path.clone().map(|path| (path, self.position()));
        self.library_scan = Some(library::scan(ctx.clone(), current));
    }

    /// Adds books that finished loading in the background scan.
    fn poll_library(&mut self) {
        let Some(scan) = &self.library_scan else { return };
        loop {
            match scan.try_recv() {
                Ok(entry) => self.library.push(entry),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.library_scan = None;
                    break;
                }
            }
        }
        let last = self.last_book.clone();
        self.library.sort_by_key(|e| (Some(&e.path) != last.as_ref(), !e.started, e.title.to_lowercase()));
        if self.selected.is_none() {
            self.selected = self.library.first().map(|e| e.path.clone());
        }
    }

    /// Moves the home screen highlight up (-1) or down (+1).
    fn move_selection(&mut self, step: isize) {
        if self.library.is_empty() {
            return;
        }
        let index = self.library.iter().position(|e| Some(&e.path) == self.selected.as_ref()).unwrap_or(0);
        let index = index.saturating_add_signed(step).min(self.library.len() - 1);
        self.selected = Some(self.library[index].path.clone());
        self.scroll_to_selected = true;
    }

    fn open(&mut self, path: &Path) {
        match epub::load(path) {
            Ok(book) => {
                self.save_progress();
                let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
                let chapters: Vec<Vec<Para>> = book.chapters.iter().map(Para::from_chapter).collect();
                self.chapter_chars = chapters.iter().map(|c| c.iter().map(|p| p.text.len()).sum()).collect();
                self.chapter_words = chapters.iter().map(|c| c.iter().map(|p| word_count(&p.text)).sum()).collect();
                self.chapter_titles = book
                    .chapters
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let title = c.title().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
                        if title.is_empty() { format!("Part {}", i + 1) } else { title }
                    })
                    .collect();
                self.chapter_records = progress::chapter_records(&path);
                self.menu_selected = 0;
                self.glide_y = None;
                self.book_words = self.chapter_words.iter().sum();
                self.book = Some(book);
                self.error = None;
                self.go_to_chapter(0);
                let saved_position = progress::load(&path);
                if let Some(pos) = saved_position {
                    self.restore(pos);
                }
                // Books already started before start dates were recorded get
                // the date BookTyper was first used.
                self.book_started = progress::started(&path).unwrap_or_else(|| match saved_position {
                    Some(_) => progress::first_use().unwrap_or_else(progress::now),
                    None => progress::now(),
                });
                self.saved = None;
                self.book_path = Some(path);
                self.save_progress();
                self.screen = Screen::Reading;
                self.library_scan = None;
                self.line_shown = None; // no slide when a book opens
                self.cursor_x = None;
                self.trail_tail = None;
                self.chapter_card = None;
            }
            Err(e) => self.error = Some(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    fn go_to_chapter(&mut self, index: usize) {
        let Some(book) = &self.book else { return };
        let Some(chapter) = book.chapters.get(index) else { return };
        self.chapter = index;
        self.paras = Para::from_chapter(chapter);
        self.current = 0;
        self.finished = false;
        self.chapter_stats = ChapterStats::default();
        self.pops.clear();
        self.shake = None;
        self.shimmers.clear();
        self.settles.clear();
        self.ghosts.clear();
    }

    /// Shows the end-of-chapter card for the chapter just typed, if enough of it was typed.
    fn finish_chapter(&mut self) {
        let stats = std::mem::take(&mut self.chapter_stats);
        let Some(book) = &self.book else { return };
        if stats.words < CARD_MIN_WORDS {
            return;
        }
        let title = book.chapters[self.chapter].title().map(|t| t.replace('\n', " ")).unwrap_or_default();
        let title = if title.trim().is_empty() { format!("Chapter {}", self.chapter + 1) } else { title };
        self.chapter_card = Some(ChapterCard { title, stats, shown: Instant::now() });
        self.web.start_wave();
        // Remembered for the chapter sidebar.
        self.chapter_records.insert(self.chapter, (stats.wpm(), stats.accuracy()));
        if let Some(path) = &self.book_path {
            progress::save_chapter_record(path, self.chapter, stats.wpm(), stats.accuracy());
        }
    }

    /// Jumps to a saved position. Everything before it counts as typed.
    fn restore(&mut self, pos: Position) {
        self.go_to_chapter(pos.chapter);
        if self.paras.is_empty() {
            return;
        }
        self.current = pos.para.min(self.paras.len() - 1);
        for para in &mut self.paras[..self.current] {
            para.typed = para.text.clone();
            para.counted_to = para.typed.len();
        }
        let para = &mut self.paras[self.current];
        let offset = pos.offset.min(para.text.len());
        para.typed = para.text[..offset].to_vec();
        para.counted_to = offset;
    }

    fn position(&self) -> Position {
        let offset = self.paras.get(self.current).map_or(0, |p| p.typed.len());
        Position { chapter: self.chapter, para: self.current, offset }
    }

    /// Queues the current position to be written to disk, unless it's already saved.
    fn save_progress(&mut self) {
        let Some(path) = &self.book_path else { return };
        let pos = self.position();
        if self.saved != Some(pos) {
            self.saver.save(path, pos, self.total_words, self.book_started);
            self.saved = Some(pos);
        }
    }

    /// Saves and waits for the write to finish, before the window closes.
    fn save_before_exit(&mut self) {
        self.save_progress();
        self.saver.finish();
        self.save_settings();
        if let Some(write) = self.settings_write.take() {
            let _ = write.join();
        }
    }

    fn save_settings(&mut self) {
        if !std::mem::take(&mut self.settings_changed) {
            return;
        }
        // Let the previous write finish first so an older one can't land last.
        if let Some(write) = self.settings_write.take() {
            let _ = write.join();
        }
        self.settings_write = Some(self.settings.save());
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        if self.screen != Screen::Settings {
            self.settings_back = self.screen;
            self.screen = Screen::Settings;
        }
        drop_focus(ctx);
    }

    fn close_settings(&mut self, ctx: &egui::Context) {
        self.screen = self.settings_back;
        self.save_settings();
        // The settings button keeps keyboard focus after a click, and a
        // focused button is clicked by Space, which would reopen settings.
        drop_focus(ctx);
    }

    /// Letters typed from the start of the book to the cursor.
    fn letters_into_book(&self) -> usize {
        let before: usize = self.chapter_chars[..self.chapter.min(self.chapter_chars.len())].iter().sum();
        let in_chapter: usize = self.paras[..self.current.min(self.paras.len())].iter().map(|p| p.text.len()).sum();
        before + in_chapter + self.paras.get(self.current).map_or(0, |p| p.typed.len())
    }

    /// Moves the marble background on: scrolled to how far you are, sparks moved.
    fn step_web(&mut self, ui: &egui::Ui) {
        if self.settings.background != Background::Marble {
            return;
        }
        let full = ui.painter().clip_rect();
        let goal = self.letters_into_book() as f32 * WEB_PER_LETTER;
        self.web.step(goal, full.width() / full.height(), (0.0, 1.0), &self.settings.marble);
        if self.web.busy(&self.settings.marble) {
            ui.ctx().request_repaint();
        }
    }

    /// (characters typed in this chapter, in the whole book), as 0..=1.
    fn fraction_done(&self) -> (f32, f32) {
        let done_in_chapter: usize = self.paras[..self.current].iter().map(|p| p.text.len()).sum::<usize>()
            + self.paras.get(self.current).map_or(0, |p| p.typed.len());
        let chapter_total: usize = self.paras.iter().map(|p| p.text.len()).sum();
        let book_total: usize = self.chapter_chars.iter().sum();
        let done_in_book = self.chapter_chars[..self.chapter].iter().sum::<usize>() + done_in_chapter;
        let ratio = |done: usize, total: usize| if total == 0 { 0.0 } else { done as f32 / total as f32 };
        (ratio(done_in_chapter, chapter_total), ratio(done_in_book, book_total))
    }

    fn type_char(&mut self, c: char) {
        if self.finished {
            return;
        }
        let Some(para) = self.paras.get_mut(self.current) else { return };
        let is_boundary = c == ' ' || c == '\n';
        if is_boundary && para.take_finished_word() {
            self.word_times.push_back(Instant::now());
            self.total_words += 1;
            self.chapter_stats.words += 1;
            // A word typed without a single wrong key gets a shine.
            let end = para.typed.len();
            let start = para.typed.iter().rposition(|c| c.is_whitespace()).map_or(0, |i| i + 1);
            if !self.mistake_in_word && (start..end).all(|i| para.is_correct(i)) {
                self.shimmers.push((self.current, start, end, Instant::now()));
            }
            // Every finished word settles, straightening its tilted letters.
            self.settles.push((self.current, start, end, Instant::now()));
            self.mistake_in_word = false;
        }
        match para.next_char() {
            // At the end of a paragraph, Space or Enter moves on.
            None => {
                if is_boundary {
                    if self.current + 1 == self.paras.len() {
                        self.finish_chapter();
                    }
                    self.next_para();
                }
            }
            // Enter only means something where the book has a line break.
            Some(target) if c == '\n' && target != '\n' => {}
            Some(_) => {
                para.typed.push(c);
                let index = para.typed.len() - 1;
                self.chapter_stats.keys += 1;
                if para.is_correct(index) {
                    let target = para.text[index];
                    let big = target.is_uppercase() || target.is_ascii_punctuation() || "“”‘’—–…".contains(target);
                    let height = (1.0 + HOP_HEIGHT_VARIES * self.random()) * if big { BIG_HOP } else { 1.0 };
                    let tilt = HOP_TILT * self.random();
                    self.pops.push((self.current, index, Instant::now(), height, tilt));
                    self.key_times.push_back(Instant::now());
                } else {
                    self.chapter_stats.mistakes += 1;
                    self.mistake_in_word = true;
                    self.shake = Some((self.current, index, Instant::now()));
                }
            }
        }
    }

    fn random(&mut self) -> f32 {
        next_random(&mut self.rng)
    }

    /// Counts the time since the last key as typing time, unless it was a long pause.
    fn note_key(&mut self) {
        self.chapter_stats.active += self.last_key.elapsed().min(IDLE_CAP);
        self.last_key = Instant::now();
    }

    fn backspace(&mut self, whole_word: bool) {
        if self.finished {
            self.finished = false;
            return;
        }
        // At the start of a paragraph, step back to the end of the previous one,
        // undoing the Space/Enter that moved on.
        if self.paras.get(self.current).is_some_and(|p| p.typed.is_empty()) {
            self.back_to_previous_para();
            return;
        }
        let Some(para) = self.paras.get_mut(self.current) else { return };
        let count = if whole_word {
            let spaces = para.typed.iter().rev().take_while(|c| c.is_whitespace()).count();
            let word = para.typed[..para.typed.len() - spaces].iter().rev().take_while(|c| !c.is_whitespace()).count();
            spaces + word
        } else {
            1
        };
        let now = Instant::now();
        for _ in 0..count.min(para.typed.len()) {
            // Wrong letters don't just vanish: they fall away and fade.
            let i = para.typed.len() - 1;
            if !para.is_correct(i) && !para.typed[i].is_whitespace() {
                self.ghosts.push((self.current, i, para.typed[i], now));
            }
            para.typed.pop();
        }
    }

    fn next_para(&mut self) {
        if self.current + 1 < self.paras.len() {
            self.current += 1;
        } else if self.book.as_ref().is_some_and(|b| self.chapter + 1 < b.chapters.len()) {
            self.go_to_chapter(self.chapter + 1);
        } else {
            self.finished = true;
        }
    }

    /// Moves to the end of the previous paragraph, into the previous chapter if needed.
    fn back_to_previous_para(&mut self) {
        if self.current > 0 {
            self.current -= 1;
        } else if self.chapter > 0 {
            self.go_to_chapter(self.chapter - 1);
            // That chapter was finished, so it all counts as typed.
            for para in &mut self.paras {
                para.typed = para.text.clone();
                para.counted_to = para.typed.len();
            }
            self.current = self.paras.len() - 1;
        }
    }

    fn prev_para(&mut self) {
        self.current = self.current.saturating_sub(1);
        self.finished = false;
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().next().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(&path);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_before_exit();
        }
        if ctx.input(|i| i.key_pressed(Key::F11)) {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
        }

        match self.screen {
            Screen::Home => self.home_input(ctx),
            Screen::Reading => self.reading_input(ctx),
            Screen::Settings => {
                if ctx.input(|i| i.key_pressed(Key::Escape)) {
                    self.close_settings(ctx);
                }
            }
        }
    }

    fn home_input(&mut self, ctx: &egui::Context) {
        self.poll_library();
        for event in ctx.input(|i| i.events.clone()) {
            let egui::Event::Key { key, pressed: true, .. } = event else { continue };
            match key {
                Key::ArrowUp => self.move_selection(-1),
                Key::ArrowDown => self.move_selection(1),
                Key::Enter => {
                    if let Some(path) = self.selected.clone() {
                        self.open(&path);
                    }
                }
                Key::Escape => {
                    self.save_before_exit();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Key::F3 => self.show_frame_times = !self.show_frame_times,
                _ => {}
            }
        }
    }

    fn reading_input(&mut self, ctx: &egui::Context) {
        // Tab would otherwise give keyboard focus to the Settings button, and
        // then Space would press it.
        drop_focus(ctx);
        for event in ctx.input(|i| i.events.clone()) {
            // The end-of-chapter card takes no typing: Space or Enter closes
            // it (once it's been up a moment), Esc still goes to the library.
            if let Some(card) = &self.chapter_card {
                let ready = card.shown.elapsed().as_secs_f32() >= CARD_DELAY;
                match event {
                    egui::Event::Key { key: Key::Space | Key::Enter, pressed: true, .. } if ready => {
                        self.chapter_card = None;
                        self.last_key = Instant::now();
                        // The next chapter appears in place, no slide or glide.
                        self.line_shown = None;
                        self.cursor_x = None;
                        self.trail_tail = None;
                    }
                    egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                        self.chapter_card = None;
                        self.go_home(ctx);
                        return;
                    }
                    egui::Event::Key { key: Key::F3, pressed: true, .. } => self.show_frame_times = !self.show_frame_times,
                    egui::Event::Key { key: Key::Tab, pressed: true, .. } => self.toggle_chapter_menu(),
                    _ => {}
                }
                continue;
            }
            match event {
                egui::Event::Text(text) => {
                    self.note_key();
                    for c in text.chars() {
                        self.type_char(c);
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    self.note_key();
                    match key {
                        Key::F3 => self.show_frame_times = !self.show_frame_times,
                        Key::Escape if self.menu_open => self.menu_open = false,
                        Key::Escape => {
                            self.go_home(ctx);
                            return;
                        }
                        Key::Tab => self.toggle_chapter_menu(),
                        // While the chapter menu is open, Up/Down/Enter work it.
                        Key::ArrowDown if self.menu_open && !modifiers.ctrl => self.move_menu_selection(1),
                        Key::ArrowUp if self.menu_open && !modifiers.ctrl => self.move_menu_selection(-1),
                        Key::Enter if self.menu_open => self.jump_to_chapter(self.menu_selected),
                        Key::Enter => self.type_char('\n'),
                        Key::Backspace => self.backspace(modifiers.ctrl),
                        Key::ArrowRight if modifiers.ctrl => self.go_to_chapter(self.chapter + 1),
                        Key::ArrowLeft if modifiers.ctrl => self.go_to_chapter(self.chapter.saturating_sub(1)),
                        Key::ArrowDown if modifiers.ctrl => self.next_para(),
                        Key::ArrowUp if modifiers.ctrl => self.prev_para(),
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Save on every paragraph change (and on closing, in `handle_input`).
        let paragraph_changed = self.saved.is_some_and(|s| (s.chapter, s.para) != (self.chapter, self.current));
        if paragraph_changed {
            self.save_progress();
        }

        while self.word_times.front().is_some_and(|t| t.elapsed() > WORD_WINDOW) {
            self.word_times.pop_front();
        }
        // A letter keeps its hop (and its tilt) while it's fresh, while its word
        // is still being typed, and while its finished word settles.
        self.settles.retain(|s| s.3.elapsed().as_secs_f32() < SETTLE_TIME);
        self.ghosts.retain(|g| g.3.elapsed().as_secs_f32() < DISSOLVE_TIME);
        self.move_streaks();
        let word = self.paras.get(self.current).map(|p| {
            let start = p.typed.iter().rposition(|c| c.is_whitespace()).map_or(0, |i| i + 1);
            start..p.typed.len()
        });
        self.pops.retain(|pop| {
            let fresh = pop.2.elapsed().as_secs_f32() < POP_TIME.max(JUMP_TIME);
            let in_word = pop.0 == self.current && word.as_ref().is_some_and(|w| w.contains(&pop.1));
            let settling = self.settles.iter().any(|s| s.0 == pop.0 && (s.1..s.2).contains(&pop.1));
            fresh || in_word || settling
        });
        self.shimmers.retain(|(.., at)| at.elapsed().as_secs_f32() < SHIMMER_TIME);
    }

    /// The library's and Settings' top bar. Returns true when the
    /// Settings/Back button was clicked. (Reading has the stats bar instead.)
    fn top_bar(&self, ui: &mut egui::Ui) -> bool {
        ui.horizontal(|ui| {
            ui.strong("BookTyper");
            let label = if self.screen == Screen::Settings { "Back" } else { "Settings" };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.button(label).clicked()).inner
        })
        .inner
    }

    fn bottom_bar(&self, ui: &mut egui::Ui) {
        ui.weak(match self.screen {
            Screen::Home => "Up/Down: choose    Enter or click: open    Drop an .epub here to add it    F11: full screen    Esc: quit",
            // Nothing while reading: the keys are listed in Settings.
            Screen::Reading => "",
            Screen::Settings => "Click a theme to use it    F11: full screen    Esc: back",
        });
    }

    fn book_text(&mut self, ui: &mut egui::Ui) {
        self.step_web(ui);
        let rect = ui.max_rect();
        let painter = ui.painter();
        let theme = self.settings.theme;
        if self.chapter_card.is_some() {
            let middle = rect.top() + HEADER_HEIGHT + (rect.height() - HEADER_HEIGHT) / 2.0;
            self.background(painter, rect, middle, 0.0, &self.web);
            self.chapter_card_view(ui, rect);
            return;
        }
        if self.book.is_none() || self.finished {
            let msg = match (&self.error, self.finished) {
                (Some(error), _) => error.as_str(),
                (None, true) => "The End",
                (None, false) => "Drop an .epub file here",
            };
            painter.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(28.0), theme.text);
            return;
        }

        // Lay out the paragraphs around the current one wrapped to the column
        // width, then show just a few of those rows: the cursor's row in the
        // middle, finished rows above it and upcoming rows below.
        let width = (rect.width() - 80.0).min(MAX_LINE_WIDTH);
        let (mut job, cursor_index, lifted) = self.page_job();
        job.wrap.max_width = width;
        let galley = painter.layout_job(job);
        // At a wrap point, put the cursor at the start of the next row rather
        // than the end of this one.
        let cursor = CCursor { prefer_next_row: true, ..CCursor::new(cursor_index) };
        let row = galley.layout_from_cursor(cursor).row;
        let gap = galley.rows[row].rect().height() * LINE_GAP;
        let left = rect.center().x - width / 2.0;
        // The cursor's line sits in the middle of the space below the header.
        let middle = rect.top() + HEADER_HEIGHT + (rect.height() - HEADER_HEIGHT) / 2.0;
        let text_area = Rect::from_x_y_ranges(rect.x_range(), rect.top() + HEADER_HEIGHT..=rect.bottom());

        // When the cursor moves to another line of the book, slide the lines
        // from where they were to their new places instead of jumping.
        let para_start = cursor_index - self.paras[self.current].typed.len();
        let row_in_para = row - galley.layout_from_cursor(CCursor::new(para_start)).row;
        let line = (self.chapter, self.current, row_in_para);
        let slide = self.slide_offset();
        if let Some(shown) = self.line_shown
            && shown != line
        {
            // Moving on: everything starts one line lower and slides up.
            let step = if line > shown { gap } else { -gap };
            if line > shown {
                self.line_flash = Some((line, Instant::now()));
            }
            self.slide_from = slide + step;
            self.scroll += step;
            self.slide_start = Instant::now();
        }
        self.line_shown = Some(line);
        let slide = self.slide_offset();
        if slide != 0.0 {
            ui.ctx().request_repaint();
        }

        self.background(painter, rect, middle, slide, &self.web);

        let shaking = self.shake.is_some_and(|(.., at)| at.elapsed().as_secs_f32() < SHAKE_TIME);
        // How far through its flash the line just finished is, while it flashes.
        let flash = self
            .line_flash
            .filter(|(after, _)| *after == line)
            .map(|(_, at)| at.elapsed().as_secs_f32() / FLASH_TIME)
            .filter(|t| *t < 1.0);
        if shaking || flash.is_some() || !self.pops.is_empty() || !self.shimmers.is_empty() || !self.settles.is_empty() || !self.ghosts.is_empty() {
            ui.ctx().request_repaint();
        }
        // Letters that are hopping or shaking are left out of the galley and
        // drawn on their own, moved: (row, spot in the galley, offset, letter).
        let lifted: Vec<_> = lifted
            .into_iter()
            .map(|letter| {
                let at = CCursor { prefer_next_row: true, ..CCursor::new(letter.index) };
                let row = galley.layout_from_cursor(at).row;
                let spot = galley.pos_from_cursor(at).min.to_vec2();
                let lay_out = |size: f32| {
                    let mut format = letter.format.clone();
                    format.font_id.size = size;
                    let mut job = LayoutJob::single_section(letter.c.to_string(), format);
                    job.wrap.max_width = f32::INFINITY;
                    painter.layout_job(job)
                };
                let normal = lay_out(letter.format.font_id.size);
                let grown = lay_out(letter.format.font_id.size * letter.scale);
                // Grow and lean around the middle of the letter's baseline, so it
                // stays planted instead of drifting sideways.
                let pivot = |g: &Galley| vec2(g.size().x / 2.0, baseline(g));
                let lean = egui::emath::Rot2::from_angle(letter.angle);
                let place = spot + pivot(&normal) + letter.offset - lean * pivot(&grown);
                (row, place, letter.angle, grown, letter.ink)
            })
            .collect();

        // Lay the rows out flat (the cursor's row at `middle`, then one `gap`
        // per row) and bend that onto the cylinder.
        let drum = Drum { middle, center_x: left + width / 2.0, radius: gap / DRUM_LINE_ANGLE, theme_text: theme.text, accent: theme.cursor };
        let uv_scale = ui.ctx().fonts(|f| {
            let [w, h] = f.font_image_size();
            vec2(1.0 / w as f32, 1.0 / h as f32)
        });
        let mut mesh = egui::Mesh::default();
        let mut cursor_origin = None;
        // One extra row on each side, for lines sliding in and out of view.
        for slot in -(LINES_ABOVE as isize + 1)..=LINES_BELOW as isize + 1 {
            let Some(row_index) = row.checked_add_signed(slot).filter(|r| *r < galley.rows.len()) else { continue };
            let flat_center = middle + slot as f32 * gap + slide;
            if ((flat_center - middle) / drum.radius).abs() > DRUM_EDGE + DRUM_LINE_ANGLE {
                continue;
            }
            // Where the galley's top-left would be for this row to sit flat in its slot.
            let origin = pos2(left, flat_center - galley.rows[row_index].rect().center().y);
            let row_flash = if slot == -1 { flash } else { None };
            drum.add_row(&mut mesh, &galley.rows[row_index], origin.to_vec2(), egui::emath::Rot2::IDENTITY, uv_scale, row_flash, None);
            for (_, place, angle, letter, ink) in lifted.iter().filter(|l| l.0 == row_index) {
                let lean = egui::emath::Rot2::from_angle(*angle);
                let ink = (*ink < 1.0).then_some((*ink, theme.upcoming()));
                for letter_row in &letter.rows {
                    drum.add_row(&mut mesh, letter_row, origin.to_vec2() + *place, lean, uv_scale, None, ink);
                }
            }
            if slot == 0 {
                cursor_origin = Some(origin);
            }
        }
        // The cursor glides along its line; its trail goes behind the text.
        let glided = cursor_origin.map(|origin| {
            let at = galley.pos_from_cursor(cursor);
            let x = self.glide_cursor(at.min.x, ui.ctx());
            self.cursor_trail_mesh(painter, &drum, origin, at, x, slide);
            (origin, at, x)
        });
        painter.with_clip_rect(text_area).add(egui::Shape::mesh(mesh));

        // Solid for a moment after each keypress, then blinks.
        let idle = self.last_key.elapsed().as_secs_f32();
        let phase = idle % BLINK_PERIOD;
        if let Some((origin, at, x)) = glided {
            if idle < BLINK_PERIOD || phase < BLINK_PERIOD / 2.0 {
                // On the cylinder too (it's on the front-most line, so barely bent).
                let (top, _) = drum.place(pos2(origin.x + x - 1.0, origin.y + at.min.y));
                let (bottom, _) = drum.place(pos2(origin.x + x - 1.0, origin.y + at.max.y));
                let bar = Rect::from_min_max(top, pos2(top.x + 2.5, bottom.y));
                painter.rect_filled(bar, 1.0, theme.cursor);
            }
        }
        let next_toggle = BLINK_PERIOD / 2.0 - phase % (BLINK_PERIOD / 2.0);
        ui.ctx().request_repaint_after(Duration::from_secs_f32(next_toggle));
    }

    /// A smear of the cursor's own movement, drawn behind the text: its tail
    /// chases the drawn cursor a little slower than the cursor moves, so it
    /// stretches out while the cursor glides (backward too, when you
    /// backspace) and shrinks back into the cursor when it stops.
    fn cursor_trail_mesh(&mut self, painter: &egui::Painter, drum: &Drum, origin: egui::Pos2, at: Rect, x: f32, slide: f32) {
        if !self.settings.trail {
            self.trail_tail = None;
            return;
        }
        // A new line starts it at the cursor.
        let dt = self.trail_frame.elapsed().as_secs_f32().min(1.0 / 30.0);
        self.trail_frame = Instant::now();
        let lag = TRAIL_LAG * self.settings.trail_length;
        let tail = match self.trail_tail {
            Some(tail) if slide == 0.0 => tail + (x - tail) * (1.0 - (-dt / lag).exp()),
            _ => x,
        };
        self.trail_tail = Some(tail);
        if (x - tail).abs() < 0.5 {
            return;
        }
        let color = self.settings.theme.cursor;
        let middle = origin.y + at.center().y;
        let mut mesh = egui::Mesh::default();
        const STEPS: u32 = 12;
        for i in 0..=STEPS {
            // 0 at the tail, 1 at the cursor.
            let t = i as f32 / STEPS as f32;
            let px = tail + (x - tail) * t;
            // Same height all along (as tall as the cursor); only the brightness fades.
            let half = at.height() / 2.0;
            let tint = color.gamma_multiply((t.powf(1.5) * TRAIL_ALPHA * self.settings.trail_brightness).min(1.0));
            let (top, _) = drum.place(pos2(origin.x + px, middle - half));
            let (bottom, _) = drum.place(pos2(origin.x + px, middle + half));
            mesh.colored_vertex(top, tint);
            mesh.colored_vertex(bottom, tint);
            if i > 0 {
                let v = 2 * i;
                mesh.add_triangle(v - 2, v - 1, v);
                mesh.add_triangle(v - 1, v, v + 1);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.ctx().request_repaint();
    }

    /// Moves the drawn cursor part of the way toward `target` (its spot along
    /// the line) and returns where to draw it. It only ever moves sideways:
    /// moving to a new line is handled by the line slide.
    fn glide_cursor(&mut self, target: f32, ctx: &egui::Context) -> f32 {
        // Capped so the first frame after a pause doesn't jump straight there.
        let dt = self.cursor_frame.elapsed().as_secs_f32().min(1.0 / 60.0);
        self.cursor_frame = Instant::now();
        let x = match self.cursor_x {
            Some(x) if (target - x).abs() > 0.5 => x + (target - x) * (1.0 - (-dt / GLIDE_TIME).exp()),
            _ => target,
        };
        self.cursor_x = Some(x);
        if x != target {
            ctx.request_repaint();
        }
        x
    }

    /// "Chapter complete" with the chapter's wpm, accuracy, time and words,
    /// fading in where the text was.
    fn chapter_card_view(&self, ui: &egui::Ui, rect: Rect) {
        let Some(card) = &self.chapter_card else { return };
        let painter = ui.painter();
        let theme = self.settings.theme;
        let age = card.shown.elapsed().as_secs_f32();
        let t = (age / CARD_FADE).min(1.0);
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        let fade = |color: Color32| color.gamma_multiply(ease);
        let rise = (1.0 - ease) * 16.0;
        let x = rect.center().x;
        let middle = rect.top() + HEADER_HEIGHT + (rect.height() - HEADER_HEIGHT) / 2.0 + rise;

        painter.text(pos2(x, middle - 120.0), Align2::CENTER_BOTTOM, "Chapter complete", FontId::proportional(16.0), fade(theme.upcoming()));
        let mut title = LayoutJob::simple_singleline(card.title.clone(), ui_font(34.0, true, self.serif), fade(theme.text));
        title.wrap = TextWrapping::truncate_at_width((rect.width() - 80.0).min(MAX_LINE_WIDTH));
        let title = painter.layout_job(title);
        painter.galley(pos2(x - title.size().x / 2.0, middle - 110.0), title, theme.text);

        let stats = card.stats;
        let columns = [
            (stats.wpm().to_string(), "wpm"),
            (format_accuracy(stats.accuracy()), "accuracy"),
            (format_seconds(stats.active.as_secs()), "time"),
            (format_thousands(stats.words as u64), "words"),
        ];
        const COLUMN: f32 = 200.0;
        let first = x - COLUMN * (columns.len() as f32 - 1.0) / 2.0;
        for (i, (value, label)) in columns.into_iter().enumerate() {
            let cx = first + i as f32 * COLUMN;
            painter.text(pos2(cx, middle + 30.0), Align2::CENTER_BOTTOM, value, ui_font(44.0, true, self.serif), fade(theme.text));
            painter.text(pos2(cx, middle + 36.0), Align2::CENTER_TOP, label, FontId::proportional(15.0), fade(theme.upcoming()));
        }

        // The hint appears once Space will actually work.
        if age >= CARD_DELAY {
            let hint_fade = ((age - CARD_DELAY) / CARD_FADE).min(1.0);
            let color = theme.upcoming().gamma_multiply(hint_fade);
            painter.text(pos2(x, middle + 120.0), Align2::CENTER_TOP, "Space to continue", FontId::proportional(16.0), color);
        }
        if age < CARD_DELAY + CARD_FADE {
            ui.ctx().request_repaint();
        }
    }

    /// How far the lines still are from their resting place, mid-slide.
    fn slide_offset(&self) -> f32 {
        let t = (self.slide_start.elapsed().as_secs_f32() / SLIDE_TIME).min(1.0);
        // Ease out with a slight overshoot ("ease out back"): quick start,
        // goes a hair past its place, settles back.
        let u = t - 1.0;
        let eased = 1.0 + (SLIDE_SPRING + 1.0) * u * u * u + SLIDE_SPRING * u * u;
        self.slide_from * (1.0 - eased)
    }

    /// The background chosen in Settings, then the vignette darkening the edges.
    /// `web` is the marble to draw, if that's the choice (the settings
    /// preview has its own).
    fn background(&self, painter: &egui::Painter, rect: Rect, middle: f32, slide: f32, web: &marble::Web) {
        // The whole panel, margins included, so there's no lighter frame at the edges.
        let full = painter.clip_rect();
        match self.settings.background {
            Background::Dots => self.background_dots(painter, rect, middle, slide),
            Background::Grain => self.paper_grain(painter, full, slide),
            Background::Streaks => self.draw_streaks(painter, full, middle),
            Background::Marble if self.marble_ready => {
                // Scaled to the window's height, so a small preview shows a
                // piece of the full-size web.
                let height = painter.ctx().content_rect().height().max(full.height());
                painter.add(web.shape(full, middle, height, &self.settings.theme, &self.settings.marble));
            }
            Background::Marble | Background::Plain => {}
        }
        vignette(painter, full);
    }

    /// Works out how fast you're typing right now and moves the speed
    /// streaks toward you that fast. Called every frame while reading.
    fn move_streaks(&mut self) {
        const WINDOW: f32 = 1.5;
        while self.key_times.front().is_some_and(|t| t.elapsed().as_secs_f32() > WINDOW) {
            self.key_times.pop_front();
        }
        let dt = self.streak_frame.elapsed().as_secs_f32().min(0.1);
        self.streak_frame = Instant::now();
        let target = self.key_times.len() as f32 / WINDOW / STREAK_FULL_SPEED;
        self.speed += (target - self.speed) * (1.0 - (-dt / STREAK_EASE).exp());
        if self.speed < 0.002 {
            self.speed = 0.0;
        }
        if self.streaks.is_empty() {
            let mut rng = self.rng;
            self.streaks = (0..STREAK_COUNT)
                .map(|_| [next_random(&mut rng), next_random(&mut rng), (next_random(&mut rng) + 1.0) / 2.0 + 0.05])
                .collect();
            self.rng = rng;
        }
        let step = dt * self.speed * 0.5;
        for i in 0..self.streaks.len() {
            self.streaks[i][2] -= step;
            // Gone past you: start again far away somewhere else.
            if self.streaks[i][2] < 0.05 {
                self.streaks[i] = [self.random(), self.random(), 1.0];
            }
        }
    }

    /// Particles flying toward you out of the middle of the screen. Each is a
    /// short streak that gets longer and brighter the faster you type; when
    /// you stop they hang still as faint specks.
    fn draw_streaks(&self, painter: &egui::Painter, rect: Rect, middle: f32) {
        let center = pos2(rect.center().x, middle);
        let spread = rect.width().max(rect.height()) * 0.12;
        let project = |p: &[f32; 3], depth: f32| center + vec2(p[0], p[1]) * spread / depth;
        let base = self.settings.theme.text;
        // Below the stats bar only.
        let painter = painter.with_clip_rect(Rect::from_x_y_ranges(rect.x_range(), rect.top() + HEADER_HEIGHT..=rect.bottom()));
        for p in &self.streaks {
            let near = 1.0 - p[2];
            let head = project(p, p[2]);
            let tail = project(p, p[2] + 0.015 + self.speed * 0.06);
            // Kept out of the way of the line you're typing.
            let clear = ((head.y - middle).abs() / 90.0).clamp(0.0, 1.0);
            let alpha = near * near * (0.05 + 0.13 * self.speed.min(1.2)) * clear;
            let width = 0.6 + 1.2 * near;
            painter.line_segment([tail, head], Stroke::new(width, base.gamma_multiply(alpha)));
        }
        if self.speed > 0.0 {
            painter.ctx().request_repaint();
        }
    }

    /// Faint random specks in the text color, like paper. They drift with the
    /// text the way the dots do.
    fn paper_grain(&self, painter: &egui::Painter, rect: Rect, slide: f32) {
        let tile = GRAIN_SIZE as f32;
        let drift = (slide - self.scroll) * DOT_PARALLAX / tile;
        let uv = Rect::from_min_size(pos2(0.0, -drift), vec2(rect.width() / tile, rect.height() / tile));
        painter.image(self.grain.id(), rect, uv, self.settings.theme.text.gamma_multiply(GRAIN_STRENGTH));
    }

    /// A faint grid of dots behind the text that drifts up as you move
    /// through the book, slower than the text, and fades toward the edges.
    fn background_dots(&self, painter: &egui::Painter, rect: Rect, middle: f32, slide: f32) {
        let color = self.settings.theme.shade(0.08);
        let drift = ((slide - self.scroll) * DOT_PARALLAX).rem_euclid(DOT_SPACING);
        let columns = (rect.width() / DOT_SPACING / 2.0).ceil() as i32;
        let half_height = rect.height() / 2.0;
        let mut y = rect.top() + drift - DOT_SPACING;
        while y < rect.bottom() + DOT_SPACING {
            let away = ((y - middle).abs() / half_height).min(1.0);
            let strength = 1.0 - away * away;
            if strength > 0.0 {
                for column in -columns..=columns {
                    let x = rect.center().x + column as f32 * DOT_SPACING;
                    painter.circle_filled(pos2(x, y), 1.3, color.gamma_multiply(strength));
                }
            }
            y += DOT_SPACING;
        }
    }

    /// The current paragraph with enough paragraphs around it to fill the
    /// lines above and below (each paragraph is at least one line), plus the
    /// character index of the cursor within it.
    fn page_job(&self) -> (LayoutJob, usize, Vec<Lifted>) {
        let pops = self
            .pops
            .iter()
            .map(|&(para, index, at, height, tilt)| (para, index, at.elapsed().as_secs_f32(), height, tilt))
            .collect();
        let shake = self.shake.map(|(para, index, at)| (para, index, at.elapsed().as_secs_f32()));
        let shimmers =
            self.shimmers.iter().map(|&(para, start, end, at)| (para, start, end, at.elapsed().as_secs_f32())).collect();
        let ghosts = self.ghosts.iter().map(|&(para, index, c, at)| (para, index, c, at.elapsed().as_secs_f32())).collect();
        let settles =
            self.settles.iter().map(|&(para, start, end, at)| (para, start, end, at.elapsed().as_secs_f32())).collect();
        let mut line = Line {
            job: LayoutJob::default(),
            chars: 0,
            serif: self.serif,
            theme: self.settings.theme,
            hop: self.settings.hop,
            shake_strength: self.settings.shake,
            pops,
            shake,
            shimmers,
            settles,
            ghosts,
            lifted: Vec::new(),
        };
        let first = self.current.saturating_sub(LINES_ABOVE);
        let last = (self.current + LINES_BELOW).min(self.paras.len() - 1);
        let mut cursor = 0;
        for i in first..=last {
            if i == self.current {
                cursor = line.chars + self.paras[i].typed.len();
            }
            line.push_para(&self.paras[i], i);
            if i != last {
                line.push_paragraph_end();
            }
        }
        (line.job, cursor, line.lifted)
    }
}

impl App {
    fn home_screen(&mut self, ui: &mut egui::Ui) {
        const ROW_HEIGHT: f32 = 110.0;
        const BAR_WIDTH: f32 = 160.0;
        let mut clicked = None;
        let theme = self.settings.theme;
        let scroll_to_selected = std::mem::take(&mut self.scroll_to_selected);

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = ui.available_width().min(COLUMN_WIDTH);
            let margin = (ui.available_width() - width) / 2.0;
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(32.0);
                    let heading =
                        ui.label(egui::RichText::new("Your books").font(ui_font(30.0, true, self.serif)).color(theme.text));
                    // All-time words typed, on the right of the heading.
                    let right = heading.rect.left() + width - 18.0;
                    ui.painter().text(
                        pos2(right, heading.rect.center().y + 4.0),
                        Align2::RIGHT_BOTTOM,
                        format_thousands(self.total_words),
                        ui_font(30.0, true, self.serif),
                        theme.text,
                    );
                    ui.painter().text(
                        pos2(right, heading.rect.center().y + 6.0),
                        Align2::RIGHT_TOP,
                        "words typed",
                        FontId::proportional(14.0),
                        theme.upcoming(),
                    );
                    ui.add_space(16.0);
                    if let Some(error) = &self.error {
                        ui.colored_label(WRONG, error);
                        ui.add_space(8.0);
                    }
                    if self.library.is_empty() {
                        ui.weak(if self.library_scan.is_some() {
                            "Looking for books..."
                        } else {
                            "No .epub files in Downloads, Documents or Desktop. Drop one onto this window."
                        });
                    }

                    for entry in &self.library {
                        let (row, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
                        let is_selected = self.selected.as_ref() == Some(&entry.path);
                        if is_selected || response.hovered() {
                            let fill = if is_selected { theme.shade(0.12) } else { theme.shade(0.06) };
                            ui.painter().rect_filled(row, 8.0, fill);
                        }
                        // The cover, or a plain card with the title's first letter.
                        let cover = Rect::from_min_size(
                            pos2(row.left() + 14.0, row.center().y - library::COVER_HEIGHT / 2.0),
                            vec2(library::COVER_WIDTH, library::COVER_HEIGHT),
                        );
                        match &entry.cover {
                            Some(texture) => {
                                // Keep the cover's shape inside the slot.
                                let size = texture.size_vec2() * (cover.width() / texture.size_vec2().x).min(cover.height() / texture.size_vec2().y);
                                let shown = Rect::from_center_size(cover.center(), size);
                                ui.painter().add(
                                    egui::epaint::RectShape::filled(shown, 4.0, Color32::WHITE)
                                        .with_texture(texture.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                                );
                            }
                            None => {
                                ui.painter().rect_filled(cover, 4.0, theme.shade(0.1));
                                let letter = entry.title.chars().next().unwrap_or('?').to_string();
                                ui.painter().text(cover.center(), Align2::CENTER_CENTER, letter, ui_font(30.0, true, self.serif), theme.upcoming());
                            }
                        }
                        let text_left = cover.right() + 18.0;
                        let title_width = row.right() - text_left - BAR_WIDTH - 42.0;

                        let mut title = LayoutJob::simple_singleline(entry.title.clone(), ui_font(22.0, true, self.serif), theme.text);
                        title.wrap = TextWrapping::truncate_at_width(title_width);
                        ui.painter().galley(pos2(text_left, row.top() + 28.0), ui.painter().layout_job(title), theme.text);
                        if let Some(author) = &entry.author {
                            let mut job = LayoutJob::simple_singleline(author.clone(), FontId::proportional(15.0), theme.upcoming());
                            job.wrap = TextWrapping::truncate_at_width(title_width);
                            ui.painter().galley(pos2(text_left, row.top() + 62.0), ui.painter().layout_job(job), theme.upcoming());
                        }

                        // Percentage above a thin progress bar on the right.
                        let bar_left = row.right() - BAR_WIDTH - 18.0;
                        let label = if entry.started { format!("{:.0}%", entry.fraction * 100.0) } else { "Not started".into() };
                        let label_color = if entry.started { theme.text } else { theme.upcoming() };
                        ui.painter().text(
                            pos2(row.right() - 18.0, row.top() + 32.0),
                            Align2::RIGHT_TOP,
                            label,
                            FontId::proportional(18.0),
                            label_color,
                        );
                        let bar = Rect::from_min_size(pos2(bar_left, row.top() + 66.0), vec2(BAR_WIDTH, 5.0));
                        ui.painter().rect_filled(bar, 2.5, theme.shade(0.2));
                        let filled = Rect::from_min_size(bar.min, vec2(BAR_WIDTH * entry.fraction.clamp(0.0, 1.0), 5.0));
                        ui.painter().rect_filled(filled, 2.5, theme.cursor);

                        if response.clicked() {
                            clicked = Some(entry.path.clone());
                        }
                        if is_selected && scroll_to_selected {
                            response.scroll_to_me(Some(Align::Center));
                        }
                        ui.add_space(6.0);
                    }
                    ui.add_space(32.0);
                });
            });
        });

        if let Some(path) = clicked {
            self.open(&path);
        }
    }

    /// Theme cards (presets, then your saved ones), color pickers for the
    /// current colors, and a box to save them as a named theme.
    fn settings_screen(&mut self, ui: &mut egui::Ui) {
        const CARD_SIZE: egui::Vec2 = vec2(170.0, 100.0);
        let theme = self.settings.theme;
        let mut picked = None;
        let mut delete = None;
        let mut add = None;

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = ui.available_width().min(COLUMN_WIDTH);
            let margin = (ui.available_width() - width) / 2.0;
            ui.horizontal(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(32.0);
                    ui.label(RichText::new("Colors").font(ui_font(30.0, true, self.serif)).color(theme.text));
                    ui.add_space(16.0);

                    let presets = PRESETS.iter().map(|(name, t)| (name.to_string(), *t, false));
                    let custom = self.settings.custom.iter().map(|(name, t)| (name.clone(), *t, true));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                        for (i, (name, card_theme, is_custom)) in presets.chain(custom).enumerate() {
                            let (card, response) = ui.allocate_exact_size(CARD_SIZE, Sense::click());
                            let painter = ui.painter();
                            // Each card is drawn in its own colors.
                            painter.rect_filled(card, 10.0, card_theme.background);
                            let outline = if card_theme == theme {
                                Stroke::new(2.5, theme.cursor)
                            } else if response.hovered() {
                                Stroke::new(1.5, theme.shade(0.5))
                            } else {
                                Stroke::new(1.0, theme.shade(0.25))
                            };
                            painter.rect_stroke(card, 10.0, outline, StrokeKind::Outside);
                            let sample = painter.text(
                                card.left_top() + vec2(16.0, 12.0),
                                Align2::LEFT_TOP,
                                "Aa",
                                ui_font(34.0, false, self.serif),
                                card_theme.text,
                            );
                            let bar = Rect::from_min_size(pos2(sample.right() + 3.0, sample.top() + 4.0), vec2(2.5, sample.height() - 8.0));
                            painter.rect_filled(bar, 1.0, card_theme.cursor);
                            painter.text(
                                pos2(bar.right() + 5.0, sample.top()),
                                Align2::LEFT_TOP,
                                "bc",
                                ui_font(34.0, false, self.serif),
                                card_theme.upcoming(),
                            );
                            painter.text(
                                card.left_bottom() + vec2(16.0, -12.0),
                                Align2::LEFT_BOTTOM,
                                &name,
                                FontId::proportional(14.0),
                                card_theme.upcoming(),
                            );
                            if is_custom {
                                let corner = Rect::from_center_size(card.right_top() + vec2(-16.0, 16.0), vec2(22.0, 22.0));
                                let remove = ui.interact(corner, ui.id().with(("delete theme", i)), Sense::click());
                                if response.hovered() || remove.hovered() {
                                    let color = if remove.hovered() { WRONG } else { card_theme.upcoming() };
                                    ui.painter().text(corner.center(), Align2::CENTER_CENTER, "×", FontId::proportional(18.0), color);
                                }
                                if remove.on_hover_text("Delete").clicked() {
                                    delete = Some(name.clone());
                                    continue;
                                }
                            }
                            if response.clicked() {
                                picked = Some(card_theme);
                            }
                        }
                    });

                    ui.add_space(28.0);
                    ui.label(RichText::new("Custom").font(ui_font(22.0, true, self.serif)).color(theme.text));
                    ui.add_space(10.0);
                    let mut edited = theme;
                    egui::Grid::new("colors").spacing(vec2(16.0, 10.0)).show(ui, |ui| {
                        for (label, color) in
                            [("Text", &mut edited.text), ("Background", &mut edited.background), ("Cursor", &mut edited.cursor)]
                        {
                            ui.label(label);
                            color_picker::color_edit_button_srgba(ui, color, color_picker::Alpha::Opaque);
                            ui.end_row();
                        }
                    });
                    if edited != theme {
                        picked = Some(edited);
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut self.new_theme_name).hint_text("Name").desired_width(220.0));
                        let name = self.new_theme_name.trim();
                        if ui.add_enabled(!name.is_empty(), Button::new("Add custom")).clicked() {
                            add = Some(name.to_string());
                        }
                    });


                    ui.add_space(28.0);
                    ui.label(RichText::new("Background").font(ui_font(22.0, true, self.serif)).color(theme.text));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        for (choice, label) in [
                            (Background::Dots, "Dots"),
                            (Background::Grain, "Paper grain"),
                            (Background::Streaks, "Speed streaks"),
                            (Background::Marble, "Marble"),
                            (Background::Plain, "Plain"),
                        ] {
                            if ui.selectable_value(&mut self.settings.background, choice, label).changed() {
                                self.settings_changed = true;
                            }
                        }
                    });
                    ui.add_space(10.0);
                    self.background_preview(ui);
                    if self.settings.background == Background::Marble {
                        ui.add_space(12.0);
                        self.marble_controls(ui);
                    }

                    ui.add_space(28.0);
                    ui.label(RichText::new("Typing feel").font(ui_font(22.0, true, self.serif)).color(theme.text));
                    ui.add_space(10.0);
                    egui::Grid::new("feel").spacing(vec2(16.0, 10.0)).show(ui, |ui| {
                        for (label, value) in
                            [("Correct letter hop", &mut self.settings.hop), ("Wrong letter shake", &mut self.settings.shake)]
                        {
                            ui.label(label);
                            let slider = egui::Slider::new(value, 0.0..=MAX_STRENGTH)
                                .step_by(0.1)
                                .custom_formatter(|v, _| if v == 0.0 { "Off".into() } else { format!("{:.0}%", v * 100.0) })
                                .custom_parser(|s| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0));
                            if ui.add(slider).changed() {
                                self.settings_changed = true;
                            }
                            ui.end_row();
                        }

                        // The glow behind the cursor.
                        ui.label("Cursor trail");
                        let trail_label = if self.settings.trail { "On" } else { "Off" };
                        if ui.checkbox(&mut self.settings.trail, trail_label).changed() {
                            self.settings_changed = true;
                        }
                        ui.end_row();
                        let trail_on = self.settings.trail;
                        for (label, value) in
                            [("Trail length", &mut self.settings.trail_length), ("Trail brightness", &mut self.settings.trail_brightness)]
                        {
                            ui.add_enabled(trail_on, egui::Label::new(label));
                            let slider = egui::Slider::new(value, 0.2..=MAX_STRENGTH)
                                .step_by(0.1)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                                .custom_parser(|s| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0));
                            if ui.add_enabled(trail_on, slider).changed() {
                                self.settings_changed = true;
                            }
                            ui.end_row();
                        }
                    });

                    ui.add_space(28.0);
                    ui.label(RichText::new("Keys while reading").font(ui_font(22.0, true, self.serif)).color(theme.text));
                    ui.add_space(10.0);
                    egui::Grid::new("keys").spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                        for (keys, action) in [
                            ("Space or Enter", "next paragraph, at the end of one"),
                            ("Tab", "chapter menu (or click the chapter at the top)"),
                            ("Ctrl + Up / Down", "previous / next paragraph"),
                            ("Ctrl + Left / Right", "previous / next chapter"),
                            ("Ctrl + plus / minus", "zoom in / out"),
                            ("F11", "full screen"),
                            ("F3", "frame times"),
                            ("Esc", "close the chapter menu, or go to the library"),
                        ] {
                            ui.label(RichText::new(keys).color(theme.text));
                            ui.label(action);
                            ui.end_row();
                        }
                    });
                    ui.add_space(32.0);
                });
            });
        });

        if let Some(theme) = picked {
            self.settings.theme = theme;
            apply_theme(ui.ctx(), &theme);
            self.settings_changed = true;
        }
        if let Some(name) = delete {
            self.settings.custom.retain(|(n, _)| *n != name);
            self.settings_changed = true;
        }
        if let Some(name) = add {
            // Saving under an existing name replaces that theme.
            self.settings.custom.retain(|(n, _)| *n != name);
            self.settings.custom.push((name, self.settings.theme));
            self.new_theme_name.clear();
            self.settings_changed = true;
        }
    }

    /// A small window showing the chosen background behind a few lines of
    /// text, live, so you can see each setting as you change it. The marble
    /// in it scrolls on its own, as if someone were typing.
    fn background_preview(&mut self, ui: &mut egui::Ui) {
        const PREVIEW_HEIGHT: f32 = 240.0;
        const PREVIEW_LINES: [&str; 3] = [
            "It is a truth universally acknowledged, that a single man in",
            "possession of a good fortune, must be in want of a wife.",
            "However little known the feelings or views of such a man may be",
        ];
        const PREVIEW_TYPED: usize = 31;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PREVIEW_HEIGHT), Sense::hover());
        let theme = self.settings.theme;
        let middle = rect.center().y;
        let window = ui.ctx().content_rect().height().max(rect.height());
        if self.settings.background == Background::Marble {
            let typed = self.preview_start.elapsed().as_secs_f32() * PREVIEW_LETTERS_PER_SECOND;
            let rows = ((window - rect.height()) / 2.0 / window, (window + rect.height()) / 2.0 / window);
            self.preview_web.step(typed * WEB_PER_LETTER, rect.width() / window, rows, &self.settings.marble);
        }
        // Speed streaks and sparks move on their own.
        if matches!(self.settings.background, Background::Marble | Background::Streaks) {
            ui.ctx().request_repaint();
        }

        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, 0.0, theme.background);
        self.background(&painter, rect, middle, 0.0, &self.preview_web);
        let font = ui_font(22.0, false, self.serif);
        let gap = 44.0;
        for (i, line) in PREVIEW_LINES.iter().enumerate() {
            let y = middle + (i as f32 - 1.0) * gap;
            let left = rect.left() + 40.0;
            let color = if i == 0 { theme.text } else { theme.upcoming().gamma_multiply(1.0 - 0.3 * (i as f32 - 1.0)) };
            if i != 1 {
                painter.text(pos2(left, y), Align2::LEFT_CENTER, line, font.clone(), color);
                continue;
            }
            // The line being typed: the typed part, the cursor, the rest.
            let (done, rest) = line.split_at(PREVIEW_TYPED);
            let typed = painter.text(pos2(left, y), Align2::LEFT_CENTER, done, font.clone(), theme.text);
            painter.rect_filled(Rect::from_min_size(pos2(typed.right() + 1.0, typed.top()), vec2(2.5, typed.height())), 1.0, theme.cursor);
            painter.text(pos2(typed.right() + 5.0, y), Align2::LEFT_CENTER, rest, font.clone(), theme.upcoming());
        }
        painter.rect_stroke(rect, 10.0, Stroke::new(1.0, theme.shade(0.25)), StrokeKind::Inside);
    }

    /// The marble background's settings.
    fn marble_controls(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0}%", v * 100.0);
        let parse = |s: &str| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0);
        let mut changed = false;
        let m = &mut self.settings.marble;
        egui::Grid::new("marble").spacing(vec2(16.0, 10.0)).show(ui, |ui| {
            ui.label("Layers");
            changed |= ui.add(egui::Slider::new(&mut m.layers, 1..=MAX_LAYERS)).changed();
            ui.end_row();
            for (label, value, range) in [
                ("Each layer's speed", &mut m.ratio, 0.1..=1.0),
                ("Scroll speed", &mut m.scroll, 0.1..=4.0),
                ("Vein thickness", &mut m.thickness, 0.2..=3.0),
                ("Web size", &mut m.size, 0.4..=3.0),
            ] {
                ui.label(label);
                let slider = egui::Slider::new(value, range).step_by(0.05).custom_formatter(percent).custom_parser(parse);
                changed |= ui.add(slider).changed();
                ui.end_row();
            }
            ui.label("Veins");
            ui.horizontal(|ui| {
                for (choice, label) in [(Veins::Plain, "Plain"), (Veins::Accent, "Cursor color"), (Veins::Fade, "Cursor color fading deeper")] {
                    changed |= ui.selectable_value(&mut m.veins, choice, label).changed();
                }
            });
            ui.end_row();
            ui.label("Sparks");
            ui.horizontal(|ui| {
                for (value, label) in [(0.0, "Off"), (1.0, "Subtle"), (2.5, "Brighter")] {
                    changed |= ui.selectable_value(&mut m.sparks, value, label).changed();
                }
            });
            ui.end_row();
            ui.label("Lamp light");
            let lamp_label = if m.lamp { "On" } else { "Off" };
            changed |= ui.checkbox(&mut m.lamp, lamp_label).changed();
            ui.end_row();
        });
        ui.add_space(6.0);
        if ui.button("Play the chapter-end wave").clicked() {
            self.preview_web.start_wave();
            drop_focus(ui.ctx());
        }
        if changed {
            self.settings_changed = true;
        }
    }

    fn record_frame_time(&mut self, start: Instant, whole_frame_secs: Option<f32>) {
        let ui_ms = start.elapsed().as_secs_f32() * 1000.0;
        // eframe reports the *previous* frame's total CPU time (layout, tessellation, painting).
        let frame_ms = whole_frame_secs.unwrap_or(0.0) * 1000.0;
        self.frame_times.push_back((Instant::now(), ui_ms, frame_ms));
        while self.frame_times.front().is_some_and(|(t, ..)| t.elapsed() > FRAME_WINDOW) {
            self.frame_times.pop_front();
        }
    }

    fn frame_time_overlay(&self, ui: &egui::Ui) {
        let worst = |pick: fn(&(Instant, f32, f32)) -> f32| self.frame_times.iter().map(pick).fold(0.0, f32::max);
        let (_, ui_ms, frame_ms) = self.frame_times.back().copied().unwrap_or((Instant::now(), 0.0, 0.0));
        let fps = self.frame_times.iter().filter(|(t, ..)| t.elapsed().as_secs_f32() < 1.0).count();
        let text = format!(
            "{fps} fps (frames drawn in the last second)
ui code {ui_ms:.2} ms (worst {:.2})
whole frame {frame_ms:.2} ms (worst {:.2})
worst = last {}s",
            worst(|f| f.1),
            worst(|f| f.2),
            FRAME_WINDOW.as_secs(),
        );
        // Below the stats bar.
        let corner = ui.max_rect().right_top() + vec2(-10.0, HEADER_HEIGHT + 10.0);
        ui.painter().text(corner, Align2::RIGHT_TOP, text, FontId::monospace(14.0), self.settings.theme.upcoming());
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ctrl +/- zoom is handled by egui; remember it for next time.
        let zoom = ctx.zoom_factor();
        if (zoom - self.settings.zoom).abs() > 0.001 {
            self.settings.zoom = zoom;
            self.settings_changed = true;
        }
        self.handle_input(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let start = Instant::now();
        // Reading has no bars, only its own stats bar, so nothing breaks the page.
        if self.screen != Screen::Reading {
            let settings_clicked = Panel::top("top").show(ui, |ui| self.top_bar(ui)).inner;
            if settings_clicked {
                if self.screen == Screen::Settings {
                    self.close_settings(ui.ctx());
                } else {
                    self.open_settings(ui.ctx());
                }
            }
            Panel::bottom("bottom").show(ui, |ui| self.bottom_bar(ui));
        }
        CentralPanel::default().show(ui, |ui| {
            match self.screen {
                Screen::Home => self.home_screen(ui),
                Screen::Reading => {
                    self.book_text(ui);
                    self.stats_bar(ui);
                    self.chapter_menu(ui);
                }
                Screen::Settings => self.settings_screen(ui),
            }
            self.record_frame_time(start, frame.info().cpu_usage);
            if self.show_frame_times {
                self.frame_time_overlay(ui);
            }
        });
    }
}

/// Builds the line's text, counting characters so we know where the cursor goes.
struct Line {
    job: LayoutJob,
    chars: usize,
    serif: bool,
    theme: Theme,
    /// Settings multipliers for the hop and shake distances.
    hop: f32,
    shake_strength: f32,
    /// (paragraph, character, seconds since typed, hop height multiplier, lean)
    /// for just-typed correct letters.
    pops: Vec<(usize, usize, f32, f32, f32)>,
    /// The same for the last wrong letter.
    shake: Option<(usize, usize, f32)>,
    /// (paragraph, first character, end, seconds since) for shining words.
    shimmers: Vec<(usize, usize, usize, f32)>,
    /// (paragraph, first character, end, seconds since) for settling words.
    settles: Vec<(usize, usize, usize, f32)>,
    /// (paragraph, character, the wrong letter, seconds since) for backspaced mistakes.
    ghosts: Vec<(usize, usize, char, f32)>,
    /// Letters drawn on their own, moved, instead of in the galley.
    lifted: Vec<Lifted>,
}

/// A letter mid-hop or mid-shake.
struct Lifted {
    /// Character index in the galley.
    index: usize,
    c: char,
    format: TextFormat,
    offset: egui::Vec2,
    /// Size (1 = normal) and lean in radians, both around the letter's baseline middle.
    scale: f32,
    angle: f32,
    /// How far it has filled with color, 0 to 1 (1 = done).
    ink: f32,
}

impl Line {
    fn push(&mut self, text: &str, format: TextFormat) {
        self.chars += text.chars().count();
        self.job.append(text, 0.0, format);
    }

    /// A little space after the last word, then the next paragraph's line.
    fn push_paragraph_end(&mut self) {
        self.push("  \n", TextFormat::simple(FontId::proportional(LINE_SIZE), self.theme.upcoming()));
    }

    /// Adds the paragraph's text, colored by what's been typed, in runs of
    /// identically formatted characters.
    fn push_para(&mut self, para: &Para, para_index: usize) {
        let mut run = String::new();
        let mut run_format: Option<TextFormat> = None;
        for i in 0..para.text.len() {
            // A backspaced mistake that used to be here falls away and fades.
            for &(_, _, c, age) in self.ghosts.iter().filter(|g| g.0 == para_index && g.1 == i && g.3 < DISSOLVE_TIME) {
                let t = age / DISSOLVE_TIME;
                let mut format = text_format(para.kind, para.styles[i], self.serif);
                format.color = WRONG.gamma_multiply(1.0 - t);
                let index = self.chars + run.chars().count();
                let offset = vec2(0.0, DISSOLVE_DROP * t * t);
                self.lifted.push(Lifted { index, c, format, offset, scale: 1.0 - 0.2 * t, angle: 0.25 * t, ink: 1.0 });
            }
            let mut format = text_format(para.kind, para.styles[i], self.serif);
            let mut offset = None;
            let (mut scale, mut angle, mut ink) = (1.0, 0.0, 1.0);
            if i < para.typed.len() {
                if para.is_correct(i) {
                    // A just-typed letter flashes in the cursor color and hops up, then settles.
                    let mut tint: f32 = 0.0;
                    if let Some(&(_, _, age, height, tilt)) = self.pops.iter().rev().find(|p| p.0 == para_index && p.1 == i) {
                        let left = 1.0 - (age / POP_TIME).min(1.0);
                        tint = left * left * 0.8;
                        ink = (age / INK_TIME).min(1.0);
                        if self.hop > 0.0 {
                            // It leans into its tilt on the way up and keeps it
                            // until its word is finished.
                            let (mut rise, mut lean) = (0.0, tilt);
                            if age < JUMP_TIME {
                                // 0 -> 1 at the top of the hop -> 0 on landing.
                                let up = (age / JUMP_TIME * std::f32::consts::PI).sin();
                                rise = -JUMP_HEIGHT * self.hop * height * up;
                                scale = 1.0 + HOP_GROW * up;
                                lean = tilt * (age / (JUMP_TIME / 2.0)).min(1.0);
                            }
                            // Finished word: straighten up, dip together, spring back.
                            if let Some(&(.., settle_age)) = self.settles.iter().find(|s| s.0 == para_index && (s.1..s.2).contains(&i)) {
                                let t = (settle_age / SETTLE_TIME).min(1.0);
                                lean *= (1.0 - t) * (1.0 - t);
                                rise += SETTLE_DIP * self.hop * (t * std::f32::consts::TAU).sin() * (1.0 - t);
                            }
                            if rise != 0.0 || lean != 0.0 || scale != 1.0 {
                                offset = Some(vec2(0.0, rise));
                                angle = lean;
                            }
                        }
                        // Still filling with ink: drawn on its own even without a hop.
                        if ink < 1.0 && offset.is_none() {
                            offset = Some(egui::Vec2::ZERO);
                        }
                    }
                    // A shine sweeps left to right across a perfect word, about 3 letters wide.
                    for &(.., start, end, age) in self.shimmers.iter().filter(|s| s.0 == para_index && (s.1..s.2).contains(&i)) {
                        let center = -1.5 + ((end - start) as f32 + 3.0) * (age / SHIMMER_TIME);
                        let shine = (1.0 - ((i - start) as f32 - center).abs() / 1.5).max(0.0);
                        tint = tint.max(shine * SHIMMER_STRENGTH);
                    }
                    format.color = self.theme.text.lerp_to_gamma(self.theme.cursor, tint);
                } else {
                    format.color = WRONG;
                    format.background = WRONG_BG;
                    // Keep the highlight inside its row so it can't peek into the next line's slot.
                    format.expand_bg = 0.0;
                    // The latest wrong letter wobbles side to side three times, dying away.
                    let shaking = |s: &(usize, usize, f32)| s.0 == para_index && s.1 == i && s.2 < SHAKE_TIME;
                    if let Some((.., age)) = self.shake.filter(shaking)
                        && self.shake_strength > 0.0
                    {
                        let t = age / SHAKE_TIME;
                        let distance = SHAKE_DISTANCE * self.shake_strength;
                        offset = Some(vec2(distance * (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t), 0.0));
                    }
                }
            } else {
                format.color = self.theme.upcoming();
            }
            let shown = if para.text[i] == '\n' { ' ' } else { para.text[i] };
            if let Some(offset) = offset {
                // Drawn separately (without the red highlight, which stays put);
                // in the galley it's invisible but still takes its space.
                let index = self.chars + run.chars().count();
                let lifted_format = TextFormat { background: Color32::TRANSPARENT, ..format.clone() };
                self.lifted.push(Lifted { index, c: shown, format: lifted_format, offset, scale, angle, ink });
                format.color = Color32::TRANSPARENT;
            }

            if run_format.as_ref() != Some(&format) {
                if let Some(f) = run_format.take() {
                    self.push(&std::mem::take(&mut run), f);
                }
                run_format = Some(format);
            }
            // Line breaks would wrap the line; show them as a space (Space or Enter types them).
            run.push(shown);
        }
        if let Some(f) = run_format {
            self.push(&run, f);
        }
    }
}

fn text_format(kind: BlockKind, style: Style, serif: bool) -> TextFormat {
    let bold = style.bold || matches!(kind, BlockKind::Heading(_));
    let family = match (kind, serif, bold, style.italic) {
        (BlockKind::Preformatted, ..) => FontFamily::Monospace,
        (_, false, ..) => FontFamily::Proportional,
        (_, true, false, false) => FontFamily::Name("serif".into()),
        (_, true, true, false) => FontFamily::Name("serif-bold".into()),
        (_, true, false, true) => FontFamily::Name("serif-italic".into()),
        (_, true, true, true) => FontFamily::Name("serif-bold-italic".into()),
    };
    TextFormat {
        font_id: FontId::new(LINE_SIZE, family),
        // Without the serif fonts there's no italic font, so egui slants the regular one.
        italics: style.italic && !serif,
        ..Default::default()
    }
}

/// 1234567 -> "1,234,567".
fn format_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn word_count(text: &[char]) -> usize {
    text.split(|c| c.is_whitespace()).filter(|w| !w.is_empty()).count()
}

/// A random number from -1 to 1 (xorshift; it only needs to look random).
fn next_random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
}

/// Side of the square paper grain tile, in texels (shown at one per point).
const GRAIN_SIZE: usize = 256;
/// How strong the grain's specks are.
const GRAIN_STRENGTH: f32 = 0.07;
/// How dark the vignette gets in the corners.
const VIGNETTE_STRENGTH: f32 = 0.45;

/// A square of random specks that tiles seamlessly (each texel is random, so
/// there are no edges to line up).
fn grain_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let pixels: Vec<Color32> = (0..GRAIN_SIZE * GRAIN_SIZE)
        .map(|_| {
            // Mostly faint, a few stronger specks.
            let v = (next_random(&mut state) + 1.0) / 2.0;
            Color32::from_white_alpha((v * v * 255.0) as u8)
        })
        .collect();
    let image = egui::ColorImage::new([GRAIN_SIZE, GRAIN_SIZE], pixels);
    ctx.load_texture("paper grain", image, egui::TextureOptions::LINEAR_REPEAT)
}

/// Darkens the edges and corners of `rect` a little, drawing the eye to the middle.
fn vignette(painter: &egui::Painter, rect: Rect) {
    const STEPS: usize = 16;
    let mut mesh = egui::Mesh::default();
    for row in 0..=STEPS {
        for column in 0..=STEPS {
            let (u, v) = (column as f32 / STEPS as f32, row as f32 / STEPS as f32);
            // 0 in the middle, 1 in the corners.
            let distance = (((u * 2.0 - 1.0).powi(2) + (v * 2.0 - 1.0).powi(2)) / 2.0).sqrt();
            let t = ((distance - 0.35) / 0.65).clamp(0.0, 1.0);
            let alpha = t * t * (3.0 - 2.0 * t) * VIGNETTE_STRENGTH;
            let pos = pos2(rect.left() + u * rect.width(), rect.top() + v * rect.height());
            mesh.colored_vertex(pos, Color32::from_black_alpha((alpha * 255.0) as u8));
        }
    }
    let width = STEPS as u32 + 1;
    for row in 0..STEPS as u32 {
        for column in 0..STEPS as u32 {
            let i = row * width + column;
            mesh.add_triangle(i, i + 1, i + width);
            mesh.add_triangle(i + 1, i + width, i + width + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Bends flat text onto a giant cylinder turning toward you: the line you're
/// typing is at the front, and lines above and below curve away, flattening,
/// shrinking and fading as they go around.
struct Drum {
    /// Screen y of the front of the cylinder (the line you're typing).
    middle: f32,
    /// Screen x the text shrinks toward.
    center_x: f32,
    radius: f32,
    /// Color for text without its own.
    theme_text: Color32,
    /// The cursor color, for the finished line's flash.
    accent: Color32,
}

impl Drum {
    /// Where flat point `p` lands on screen, and how visible it is (1 at the
    /// front, 0 at `DRUM_EDGE` and beyond).
    fn place(&self, p: egui::Pos2) -> (egui::Pos2, f32) {
        self.place_in_line(p, p.y)
    }

    /// Like `place`, but the sideways shrink is taken at `line_y` (the middle
    /// of the point's line), so a whole line shrinks evenly and its letters
    /// stay upright instead of looking slanted.
    fn place_in_line(&self, p: egui::Pos2, line_y: f32) -> (egui::Pos2, f32) {
        let perspective = |y: f32| {
            let angle = ((y - self.middle) / self.radius).clamp(-DRUM_EDGE, DRUM_EDGE);
            let focal = DRUM_FOCAL * self.radius;
            (angle, focal / (focal + self.radius * (1.0 - angle.cos())))
        };
        let (angle, depth) = perspective(p.y);
        let (_, line_depth) = perspective(line_y);
        let x = self.center_x + (p.x - self.center_x) * line_depth;
        let y = self.middle + self.radius * angle.sin() * depth;
        let visible = ((angle.cos() - DRUM_EDGE.cos()) / (1.0 - DRUM_EDGE.cos())).clamp(0.0, 1.0);
        (pos2(x, y), visible.powf(1.5))
    }

    /// Adds one row of a galley to `mesh`, bent onto the cylinder. `offset` is
    /// where the galley's top-left would be on the flat page; `lean` rotates
    /// the row around that point first (for tilted hopping letters).
    /// `flash` (0 to 1) is how far through its flash a just-finished line is.
    fn add_row(
        &self,
        mesh: &mut egui::Mesh,
        row: &egui::epaint::text::PlacedRow,
        offset: egui::Vec2,
        lean: egui::emath::Rot2,
        uv_scale: egui::Vec2,
        flash: Option<f32>,
        ink: Option<(f32, Color32)>,
    ) {
        let source = &row.visuals.mesh;
        let line_y = offset.y + row.pos.y + row.size.y / 2.0;
        // The flash: the whole line glows a little, and a brighter band sweeps
        // left to right, both fading out.
        let row_left = offset.x + row.pos.x;
        let flash_at = |x: f32| {
            let Some(t) = flash else { return 0.0 };
            let sweep = row_left + row.size.x * (t * 1.4 - 0.2);
            let band = (1.0 - (x - sweep).abs() / 140.0).max(0.0);
            ((0.25 + 0.75 * band) * (1.0 - t)).min(1.0)
        };
        let first = mesh.vertices.len() as u32;
        mesh.indices.extend(source.indices.iter().map(|i| i + first));
        mesh.vertices.extend(source.vertices.iter().map(|v| {
            let flat = (offset + lean * (row.pos.to_vec2() + v.pos.to_vec2())).to_pos2();
            let (pos, visible) = self.place_in_line(flat, line_y);
            let color = if v.color == Color32::PLACEHOLDER { self.theme_text } else { v.color };
            // Ink fill: the letter takes its color from the bottom up, starting
            // from the not-yet-typed color.
            let color = match ink {
                Some((t, from)) if color.a() > 0 => {
                    let height = 1.0 - v.pos.y / row.size.y.max(1.0);
                    from.lerp_to_gamma(color, ((t * 1.3 - height) * 5.0).clamp(0.0, 1.0))
                }
                _ => color,
            };
            let color = if color.a() > 0 { color.lerp_to_gamma(self.accent, flash_at(flat.x)) } else { color };
            egui::epaint::Vertex { pos, uv: (v.uv.to_vec2() * uv_scale).to_pos2(), color: color.gamma_multiply(visible) }
        }));
    }
}

/// Distance from the top of a one-line galley to its text's baseline.
fn baseline(galley: &Galley) -> f32 {
    galley.rows.first().map_or(galley.size().y * 0.8, |row| {
        row.pos.y + row.glyphs.first().map_or(row.size.y * 0.8, |glyph| glyph.pos.y)
    })
}

/// "100%", "98.4%". Never rounds up to 100.
fn format_accuracy(percent: f32) -> String {
    if percent >= 100.0 { "100%".to_string() } else { format!("{:.1}%", percent.min(99.9)) }
}

/// When you'll finish at the rate you've been going since you first opened
/// the book: "on pace to finish Jan 22". Dates are in UTC.
fn finish_estimate(started: u64, now: u64, done: f32) -> String {
    let elapsed = now.saturating_sub(started);
    if done >= 1.0 {
        return "finished".to_string();
    }
    // Too early to guess: under an hour in, or barely started.
    if elapsed < 3600 || done < 0.001 {
        return format!("started {}", format_date(started, now));
    }
    let finish = started + (elapsed as f64 / done as f64) as u64;
    format!("on pace to finish {}", format_date(finish, now))
}

/// "today", "Jan 22", or "Jan 22, 2027" when it isn't this year.
fn format_date(seconds: u64, now: u64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = (seconds / 86_400) as i64;
    let today = (now / 86_400) as i64;
    if day == today {
        return "today".to_string();
    }
    let (year, month, date) = civil_from_days(day);
    let (this_year, ..) = civil_from_days(today);
    let name = MONTHS[month as usize - 1];
    if year == this_year { format!("{name} {date}") } else { format!("{name} {date}, {year}") }
}

/// (year, month 1-12, day 1-31) for a count of days since 1970-01-01
/// (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// "45 s", "4:05", "1:02:09".
fn format_seconds(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match (h, m) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m}:{s:02}"),
        (h, m) => format!("{h}:{m:02}:{s:02}"),
    }
}

/// Gives egui's own widgets (top bar, buttons, pickers) the theme's colors.
fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    let mut visuals = if theme.is_light() { Visuals::light() } else { Visuals::dark() };
    visuals.panel_fill = theme.background;
    visuals.extreme_bg_color = theme.shade(0.08);
    visuals.widgets.noninteractive.fg_stroke.color = theme.shade(0.75);
    ctx.set_visuals(visuals);
}

fn drop_focus(ctx: &egui::Context) {
    if let Some(id) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
}

/// Georgia for headings and titles when it's available.
fn ui_font(size: f32, bold: bool, serif: bool) -> FontId {
    match (serif, bold) {
        (false, _) => FontId::proportional(size),
        (true, false) => FontId::new(size, FontFamily::Name("serif".into())),
        (true, true) => FontId::new(size, FontFamily::Name("serif-bold".into())),
    }
}

/// Loads Georgia (regular/bold/italic/bold-italic) from Windows' font folder.
/// Each gets egui's default fonts as fallback for characters Georgia lacks.
fn install_fonts(ctx: &egui::Context) -> bool {
    let files = [
        ("serif", "georgia.ttf"),
        ("serif-bold", "georgiab.ttf"),
        ("serif-italic", "georgiai.ttf"),
        ("serif-bold-italic", "georgiaz.ttf"),
    ];
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for (name, file) in files {
        let Ok(bytes) = std::fs::read(Path::new(r"C:\Windows\Fonts").join(file)) else {
            return false;
        };
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
        let mut chain = vec![name.to_string()];
        chain.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), chain);
    }
    ctx.set_fonts(fonts);
    true
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, finish_estimate, format_seconds, format_thousands, next_random};

    #[test]
    fn dates_from_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_740), (2026, 10, 14));
        assert_eq!(civil_from_days(20_840), (2027, 1, 22));
    }

    #[test]
    fn finish_date_from_pace() {
        let day = 86_400;
        let start = 20_740 * day; // 2026-10-14
        // An eighth done after 8 days: 64 days in total, so 2026-12-17.
        assert_eq!(finish_estimate(start, start + 8 * day, 0.125), "on pace to finish Dec 17");
        // A quarter done after 25 days: 100 days in total, so 2027-01-22.
        assert_eq!(finish_estimate(start, start + 25 * day, 0.25), "on pace to finish Jan 22, 2027");
        assert_eq!(finish_estimate(start, start + 60, 0.1), "started today");
        assert_eq!(finish_estimate(start, start + 10 * day, 1.0), "finished");
    }

    #[test]
    fn random_stays_in_range_and_spreads() {
        let mut state = 12345;
        let values: Vec<f32> = (0..10_000).map(|_| next_random(&mut state)).collect();
        assert!(values.iter().all(|v| (-1.0..=1.0).contains(v)));
        assert!(values.iter().any(|&v| v < -0.9) && values.iter().any(|&v| v > 0.9));
    }

    #[test]
    fn formats_chapter_time() {
        assert_eq!(format_seconds(45), "45 s");
        assert_eq!(format_seconds(245), "4:05");
        assert_eq!(format_seconds(3729), "1:02:09");
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }

}
