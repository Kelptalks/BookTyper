//! Remembers where you left off in each book, and which book you had open last.
//!
//! Stored as plain text in %APPDATA%\BookTyper\progress.txt:
//!   last<TAB><book path>
//!   words<TAB><words typed, all books, ever>
//!   started<TAB><unix seconds when the book was first opened><TAB><book path>
//!   <chapter><TAB><paragraph><TAB><characters typed><TAB><book path>

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub chapter: usize,
    pub para: usize,
    /// Characters already typed in that paragraph.
    pub offset: usize,
}

fn progress_file() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("BookTyper").join("progress.txt"))
}

/// %APPDATA%\BookTyper\chapter-stats.txt, one line per finished chapter
/// (a later line for the same chapter wins):
///   <chapter><TAB><wpm><TAB><accuracy %><TAB><book path>
fn records_file() -> Option<PathBuf> {
    Some(progress_file()?.with_file_name("chapter-stats.txt"))
}

/// (wpm, accuracy %) for each chapter of `book` that has an end-of-chapter card.
pub fn chapter_records(book: &Path) -> HashMap<usize, (u32, f32)> {
    let text = records_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let book = book.to_string_lossy();
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let chapter = fields.next()?.parse().ok()?;
            let wpm = fields.next()?.parse().ok()?;
            let accuracy = fields.next()?.parse().ok()?;
            (fields.next()? == book).then_some((chapter, (wpm, accuracy)))
        })
        .collect()
}

/// Adds a chapter's stats to the file, on a background thread.
pub fn save_chapter_record(book: &Path, chapter: usize, wpm: u32, accuracy: f32) {
    let line = format!("{chapter}\t{wpm}\t{accuracy:.1}\t{}\n", book.to_string_lossy());
    std::thread::spawn(move || {
        let Some(file) = records_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let result = std::fs::OpenOptions::new().create(true).append(true).open(&file).and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = result {
            eprintln!("couldn't save chapter stats to {}: {e}", file.display());
        }
    });
}

fn read_lines() -> Vec<String> {
    progress_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn last_book() -> Option<PathBuf> {
    read_lines().iter().find_map(|line| line.strip_prefix("last\t").map(PathBuf::from))
}

pub fn load(book: &Path) -> Option<Position> {
    all().into_iter().find(|(path, _)| path == book).map(|(_, position)| position)
}

/// Every book with a saved position.
pub fn all() -> Vec<(PathBuf, Position)> {
    read_lines()
        .iter()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let chapter = fields.next()?.parse().ok()?;
            let para = fields.next()?.parse().ok()?;
            let offset = fields.next()?.parse().ok()?;
            Some((PathBuf::from(fields.next()?), Position { chapter, para, offset }))
        })
        .collect()
}

/// When `book` was first opened, in Unix seconds, if that's been recorded.
pub fn started(book: &Path) -> Option<u64> {
    let book = book.to_string_lossy();
    read_lines().iter().find_map(|line| {
        let mut fields = line.strip_prefix("started\t")?.splitn(2, '\t');
        let seconds = fields.next()?.parse().ok()?;
        (fields.next()? == book).then_some(seconds)
    })
}

/// When the progress file was created: the closest record of when you first
/// used BookTyper, for books opened before start dates were recorded.
pub fn first_use() -> Option<u64> {
    let created = std::fs::metadata(progress_file()?).ok()?.created().ok()?;
    Some(created.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
}

/// Now, in Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Words typed across every book, ever.
pub fn total_words() -> u64 {
    read_lines().iter().find_map(|line| line.strip_prefix("words\t")?.parse().ok()).unwrap_or(0)
}

/// Records `position` and the start date for `book`, marks it as the last
/// book opened, and stores the all-time word count.
pub fn save(book: &Path, position: Position, total_words: u64, started: u64) {
    let Some(file) = progress_file() else { return };
    let book = book.to_string_lossy();
    let mut lines: Vec<String> = read_lines()
        .into_iter()
        .filter(|line| {
            !line.starts_with("last\t") && !line.starts_with("words\t") && !line.ends_with(&format!("\t{book}"))
        })
        .collect();
    lines.insert(0, format!("last\t{book}"));
    lines.insert(1, format!("words\t{total_words}"));
    lines.push(format!("started\t{started}\t{book}"));
    lines.push(format!("{}\t{}\t{}\t{book}", position.chapter, position.para, position.offset));
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write a temporary file and swap it in, so anyone reading the progress
    // file at the same moment never sees it half-written.
    let temp = file.with_extension("tmp");
    let result = std::fs::write(&temp, lines.join("\n") + "\n").and_then(|_| std::fs::rename(&temp, &file));
    if let Err(e) = result {
        eprintln!("couldn't save progress to {}: {e}", file.display());
    }
}

/// Saves positions on a background thread, so a slow disk write (antivirus
/// scanning the file can take most of a second) never stalls typing.
/// Writes happen in the order they were asked for.
pub struct Saver {
    queue: Option<Sender<(PathBuf, Position, u64, u64)>>,
    worker: Option<JoinHandle<()>>,
}

impl Saver {
    pub fn new() -> Self {
        let (queue, jobs) = mpsc::channel::<(PathBuf, Position, u64, u64)>();
        let worker = std::thread::spawn(move || {
            while let Ok(mut job) = jobs.recv() {
                // If more saves queued up meanwhile, only the newest position
                // per book matters.
                while let Ok(next) = jobs.try_recv() {
                    if next.0 != job.0 {
                        save(&job.0, job.1, job.2, job.3);
                    }
                    job = next;
                }
                save(&job.0, job.1, job.2, job.3);
            }
        });
        Saver { queue: Some(queue), worker: Some(worker) }
    }

    pub fn save(&self, book: &Path, position: Position, total_words: u64, started: u64) {
        if let Some(queue) = &self.queue {
            let _ = queue.send((book.to_path_buf(), position, total_words, started));
        }
    }

    /// Waits until every queued save is written. Saves after this are ignored.
    pub fn finish(&mut self) {
        self.queue = None; // closing the channel ends the worker's loop
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.finish();
    }
}
