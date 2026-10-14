//! The typing state for one chapter: what you're supposed to type and what
//! you actually typed, paragraph by paragraph.

use crate::epub::{BlockKind, Book, Chapter, Style};
use crate::progress::Position;

/// One block of the book, ready to be typed.
pub struct Para {
    pub kind: BlockKind,
    /// The text to type, one entry per character.
    pub text: Vec<char>,
    /// Bold/italic for each character in `text` (same length).
    pub styles: Vec<Style>,
    /// What you've typed so far. `typed[i]` is your attempt at `text[i]`.
    pub typed: Vec<char>,
    /// `typed.len()` when we last counted a finished word, so backspacing and
    /// retyping a word doesn't count it twice.
    pub counted_to: usize,
}

impl Para {
    pub fn from_chapter(chapter: &Chapter) -> Vec<Para> {
        chapter
            .blocks
            .iter()
            .map(|block| {
                let mut text = Vec::new();
                let mut styles = Vec::new();
                for span in &block.spans {
                    for c in span.text.chars().filter(|c| !is_invisible(*c)) {
                        text.push(c);
                        styles.push(span.style);
                    }
                }
                Para { kind: block.kind, text, styles, typed: Vec::new(), counted_to: 0 }
            })
            .filter(|p| !p.text.is_empty())
            .collect()
    }

    /// The character the cursor is on, if any is left.
    pub fn next_char(&self) -> Option<char> {
        self.text.get(self.typed.len()).copied()
    }

    /// Call when a word boundary is typed. True if a new word was just finished.
    pub fn take_finished_word(&mut self) -> bool {
        let ends_in_word = self.typed.last().is_some_and(|c| !c.is_whitespace());
        if ends_in_word && self.typed.len() > self.counted_to {
            self.counted_to = self.typed.len();
            return true;
        }
        false
    }

    pub fn is_correct(&self, i: usize) -> bool {
        matches(self.text[i], self.typed[i])
    }
}

/// How far through the book `position` is, from 0 to 1, by characters to type.
pub fn fraction_read(book: &Book, position: Position) -> f32 {
    let chapter_len = |chapter: &Chapter| Para::from_chapter(chapter).iter().map(|p| p.text.len()).sum::<usize>();
    let total: usize = book.chapters.iter().map(chapter_len).sum();
    if total == 0 {
        return 0.0;
    }
    let mut done: usize = book.chapters.iter().take(position.chapter).map(chapter_len).sum();
    if let Some(chapter) = book.chapters.get(position.chapter) {
        let paras = Para::from_chapter(chapter);
        done += paras.iter().take(position.para).map(|p| p.text.len()).sum::<usize>();
        done += paras.get(position.para).map_or(0, |p| position.offset.min(p.text.len()));
    }
    done as f32 / total as f32
}

/// Characters that take up no space on screen; nobody could type them.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}')
}

/// Does a keypress count for this character of the book? Books use a lot
/// of characters that aren't on a keyboard, so those accept the closest key.
pub fn matches(target: char, typed: char) -> bool {
    if target == typed {
        return true;
    }
    match target {
        '“' | '”' | '„' | '«' | '»' | '″' => typed == '"',
        '‘' | '’' | '‚' | '′' => typed == '\'',
        '—' | '–' | '‐' | '‑' | '−' => typed == '-',
        '…' => typed == '.',
        '\u{a0}' | '\u{2002}' | '\u{2003}' | '\u{2009}' | '\n' => typed == ' ',
        c if c.is_ascii() => false,
        // é -> e, Ç -> C ... and anything else not on a keyboard
        // (£, §, †, °) accepts any key.
        c => match base_letter(c) {
            Some(base) => typed == base,
            None => true,
        },
    }
}

fn base_letter(c: char) -> Option<char> {
    let lower = c.to_lowercase().next()?;
    let base = match lower {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'æ' => 'a',
        'ç' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'œ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ß' | 'š' => 's',
        'ž' => 'z',
        _ => return None,
    };
    Some(if c.is_uppercase() { base.to_ascii_uppercase() } else { base })
}
