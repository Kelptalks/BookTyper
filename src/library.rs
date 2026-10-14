//! The home screen's list of books: every .epub in the usual folders, plus
//! any book opened before (wherever it lives).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::epub;
use crate::progress::{self, Position};
use crate::typing;

pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub author: Option<String>,
    /// How far through the book you are, 0 to 1.
    pub fraction: f32,
    /// Has a saved position.
    pub started: bool,
    /// The book's cover art, if it has one.
    pub cover: Option<egui::TextureHandle>,
}

/// Size of the cover shown on the home screen, in points.
pub const COVER_WIDTH: f32 = 60.0;
pub const COVER_HEIGHT: f32 = 90.0;

/// Folders searched for .epub files (not their subfolders).
fn book_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for name in ["Downloads", "Documents", "Desktop"] {
            folders.push(home.join(name));
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        folders.push(dir.join("books"));
    }
    folders
}

/// Finds and loads every book on a background thread. Entries arrive on the
/// returned channel one at a time; it disconnects when the scan is done.
/// `current` overrides the saved position of the book that was just open,
/// in case its latest save hasn't reached the disk yet.
pub fn scan(ctx: egui::Context, current: Option<(PathBuf, Position)>) -> Receiver<Entry> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut saved = progress::all();
        if let Some((path, position)) = current {
            saved.retain(|(p, _)| *p != path);
            saved.push((path, position));
        }

        let mut paths: Vec<PathBuf> = saved.iter().map(|(p, _)| p.clone()).collect();
        for folder in book_folders() {
            let Ok(dir) = std::fs::read_dir(folder) else { continue };
            for file in dir.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub")) {
                    paths.push(std::path::absolute(&path).unwrap_or(path));
                }
            }
        }

        // Windows paths aren't case-sensitive, so compare them lowercased.
        let mut seen = HashSet::new();
        for path in paths {
            if !seen.insert(path.to_string_lossy().to_lowercase()) {
                continue;
            }
            // Books that fail to load (DRM, broken files) just don't show up.
            let Ok(book) = epub::load(&path) else { continue };
            let position = saved.iter().find(|(p, _)| *p == path).map(|(_, pos)| *pos);
            let entry = Entry {
                title: book.title.clone().unwrap_or_else(|| {
                    path.file_stem().map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
                }),
                author: book.author.clone(),
                fraction: position.map_or(0.0, |pos| typing::fraction_read(&book, pos)),
                started: position.is_some(),
                // Twice the size it's shown at, so it stays sharp on high-DPI screens.
                cover: epub::cover(&path, 2 * COVER_WIDTH as u32, 2 * COVER_HEIGHT as u32).map(|(w, h, pixels)| {
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                    ctx.load_texture(format!("cover {}", path.display()), image, egui::TextureOptions::LINEAR)
                }),
                path,
            };
            if sender.send(entry).is_err() {
                return; // the home screen went away
            }
            ctx.request_repaint();
        }
        ctx.request_repaint();
    });
    receiver
}
