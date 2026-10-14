//! EPUB loading: turns an .epub file into chapters of styled text.
//!
//! An .epub is a zip. `META-INF/container.xml` points at the package file
//! (`.opf`), whose `<manifest>` lists every file and whose `<spine>` lists the
//! chapters in reading order. Each chapter is an XHTML page, which we flatten
//! into blocks (headings, paragraphs, ...) made of spans (bold/italic runs).
//! See README.md for the longer version.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};
use zip::ZipArchive;

pub struct Book {
    pub title: Option<String>,
    pub author: Option<String>,
    pub chapters: Vec<Chapter>,
}

/// One spine entry (one XHTML file). Publishers split files however they
/// like, so this is *usually* a chapter, but can be a cover page, a title
/// page, or half a chapter.
pub struct Chapter {
    /// Path of the XHTML file inside the zip.
    #[allow(dead_code)] // handy for debugging, not shown anywhere yet
    pub path: String,
    pub blocks: Vec<Block>,
}

impl Chapter {
    /// Text of the first heading, if the chapter has one.
    pub fn title(&self) -> Option<String> {
        self.blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Heading(_)))
            .map(Block::text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// `<h1>`..`<h6>`; the number is the level, 1 = biggest.
    Heading(u8),
    Paragraph,
    /// Text inside `<blockquote>` (letters, epigraphs, poems...).
    Quote,
    ListItem,
    /// `<pre>`: whitespace and newlines are kept exactly.
    Preformatted,
}

/// A paragraph-like chunk of text. Starts a new line when displayed.
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
}

impl Block {
    /// The block's text with styling dropped.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// A run of text that shares one style. Concatenating a block's spans gives
/// its full text; spaces between words are already included.
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Zip(zip::result::ZipError),
    Xml { file: String, err: roxmltree::Error },
    Missing(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Zip(e) => write!(f, "zip: {e}"),
            Error::Xml { file, err } => write!(f, "bad xml in {file}: {err}"),
            Error::Missing(what) => write!(f, "missing {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::Zip(e)
    }
}

pub fn load(path: impl AsRef<Path>) -> Result<Book, Error> {
    let mut zip = ZipArchive::new(File::open(path)?)?;

    // 1. container.xml -> where the .opf lives
    let container = read_file(&mut zip, "META-INF/container.xml")?;
    let container = parse_xml(&container, "META-INF/container.xml")?;
    let opf_path = container
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| Error::Missing("rootfile in META-INF/container.xml".into()))?
        .to_string();

    // 2. the .opf -> metadata, manifest (id -> file), spine (reading order)
    let opf_text = read_file(&mut zip, &opf_path)?;
    let opf = parse_xml(&opf_text, &opf_path)?;
    let opf_dir = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    let title = metadata_text(&opf, "title");
    let author = metadata_text(&opf, "creator");

    let manifest: HashMap<&str, &str> = opf
        .descendants()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
        .collect();

    // 3. each spine entry -> blocks of styled text
    let mut chapters = Vec::new();
    for itemref in opf.descendants().filter(|n| n.has_tag_name("itemref")) {
        let Some(href) = itemref.attribute("idref").and_then(|id| manifest.get(id)) else {
            continue;
        };
        let path = resolve_href(opf_dir, href);
        let xhtml = read_file(&mut zip, &path)?;
        let blocks = parse_chapter(&xhtml, &path)?;
        if !blocks.is_empty() {
            chapters.push(Chapter { path, blocks });
        }
    }

    Ok(Book { title, author, chapters })
}

/// The book's cover image, shrunk to fit `max_width` x `max_height`, as
/// (width, height, RGBA pixels). None if the book has no cover we can read.
pub fn cover(path: impl AsRef<Path>, max_width: u32, max_height: u32) -> Option<(usize, usize, Vec<u8>)> {
    let mut zip = ZipArchive::new(File::open(path).ok()?).ok()?;
    let container = read_file(&mut zip, "META-INF/container.xml").ok()?;
    let container = parse_xml(&container, "META-INF/container.xml").ok()?;
    let opf_path = container.descendants().find(|n| n.has_tag_name("rootfile"))?.attribute("full-path")?.to_string();
    let opf_text = read_file(&mut zip, &opf_path).ok()?;
    let opf = parse_xml(&opf_text, &opf_path).ok()?;
    let opf_dir = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    // EPUB 3 marks the cover in the manifest; EPUB 2 names it in a <meta>;
    // failing both, take an image whose name says "cover".
    let items: Vec<Node> = opf.descendants().filter(|n| n.has_tag_name("item")).collect();
    let is_image = |n: &Node| n.attribute("media-type").is_some_and(|t| t.starts_with("image/"));
    let meta_id = opf
        .descendants()
        .find(|n| n.has_tag_name("meta") && n.attribute("name") == Some("cover"))
        .and_then(|n| n.attribute("content"));
    let item = items
        .iter()
        .find(|n| n.attribute("properties").is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image")))
        .or_else(|| items.iter().find(|n| meta_id.is_some() && n.attribute("id") == meta_id && is_image(n)))
        .or_else(|| {
            items.iter().find(|n| {
                is_image(n) && [n.attribute("id"), n.attribute("href")].iter().flatten().any(|s| s.to_lowercase().contains("cover"))
            })
        })?;
    let file_name = resolve_href(opf_dir, item.attribute("href")?);

    let mut bytes = Vec::new();
    zip.by_name(&file_name).ok()?.read_to_end(&mut bytes).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.thumbnail(max_width, max_height).to_rgba8();
    Some((image.width() as usize, image.height() as usize, image.into_raw()))
}

fn read_file<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<String, Error> {
    let mut file = zip
        .by_name(name)
        .map_err(|_| Error::Missing(format!("{name} in the epub")))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn parse_xml<'a>(text: &'a str, file: &str) -> Result<Document<'a>, Error> {
    let options = ParsingOptions { allow_dtd: true, ..Default::default() };
    Document::parse_with_options(text.trim_start_matches('\u{feff}'), options)
        .map_err(|err| Error::Xml { file: file.to_string(), err })
}

fn metadata_text(opf: &Document, tag: &str) -> Option<String> {
    let metadata = opf.descendants().find(|n| n.has_tag_name("metadata"))?;
    let node = metadata.descendants().find(|n| n.has_tag_name(tag))?;
    let text = node.text()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Manifest hrefs are URLs relative to the .opf's folder: they can contain
/// `../`, `%20`-style escapes, and `#fragments`. Zip entry names are plain
/// paths from the zip root.
fn resolve_href(base_dir: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = base_dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = s.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Chapter XHTML -> blocks

fn parse_chapter(xhtml: &str, file: &str) -> Result<Vec<Block>, Error> {
    let xhtml = replace_html_entities(xhtml);
    let doc = parse_xml(&xhtml, file)?;
    let root = doc.root_element();
    let body = root.descendants().find(|n| n.has_tag_name("body")).unwrap_or(root);

    let mut builder = BlockBuilder::new();
    builder.walk(body, Style::default(), Context { kind: BlockKind::Paragraph, pre: false });
    builder.flush();
    Ok(builder.blocks)
}

/// What a block started at this point in the tree should be.
#[derive(Clone, Copy)]
struct Context {
    kind: BlockKind,
    pre: bool,
}

struct BlockBuilder {
    blocks: Vec<Block>,
    kind: BlockKind,
    spans: Vec<Span>,
    /// HTML collapses any run of whitespace into one space. We remember that
    /// whitespace was seen and only emit the space once the next word arrives,
    /// so blocks never start or end with one.
    pending_space: bool,
}

impl BlockBuilder {
    fn new() -> Self {
        BlockBuilder { blocks: Vec::new(), kind: BlockKind::Paragraph, spans: Vec::new(), pending_space: false }
    }

    fn walk(&mut self, node: Node, style: Style, ctx: Context) {
        for child in node.children() {
            if child.is_text() {
                self.push_text(child.text().unwrap_or(""), style, ctx.pre);
                continue;
            }
            if !child.is_element() {
                continue;
            }
            let name = child.tag_name().name().to_ascii_lowercase();
            match name.as_str() {
                "head" | "title" | "script" | "style" | "svg" | "math" => {}
                "br" => self.line_break(),

                "b" | "strong" => self.walk(child, Style { bold: true, ..style }, ctx),
                "i" | "em" | "cite" | "dfn" | "var" => self.walk(child, Style { italic: true, ..style }, ctx),

                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = name.as_bytes()[1] - b'0';
                    self.block(child, BlockKind::Heading(level), style, ctx);
                }
                "blockquote" => {
                    let inner = Context { kind: BlockKind::Quote, ..ctx };
                    self.block(child, BlockKind::Quote, style, inner);
                }
                "li" => {
                    let inner = Context { kind: BlockKind::ListItem, ..ctx };
                    self.block(child, BlockKind::ListItem, style, inner);
                }
                "pre" => {
                    let inner = Context { kind: BlockKind::Preformatted, pre: true };
                    self.block(child, BlockKind::Preformatted, style, inner);
                }
                "p" | "div" | "section" | "article" | "header" | "footer" | "aside" | "nav" | "main"
                | "figure" | "figcaption" | "table" | "tr" | "td" | "th" | "ul" | "ol" | "dl" | "dt"
                | "dd" | "hr" | "address" | "center" => self.block(child, ctx.kind, style, ctx),

                // span, a, small, sup, ... : no effect we track, just keep the text
                _ => self.walk(child, style, ctx),
            }
        }
    }

    fn block(&mut self, node: Node, kind: BlockKind, style: Style, ctx: Context) {
        self.flush();
        self.kind = kind;
        self.walk(node, style, ctx);
        self.flush();
        // Loose text after this element belongs to the surrounding block type.
        self.kind = ctx.kind;
    }

    fn push_text(&mut self, text: &str, style: Style, pre: bool) {
        for c in text.chars() {
            if pre {
                self.push_char(c, style);
            } else if c.is_whitespace() && c != '\u{a0}' {
                self.pending_space = true;
            } else {
                let after_break = self.spans.last().is_some_and(|s| s.text.ends_with('\n'));
                if self.pending_space && self.has_text() && !after_break {
                    // Join the space onto the previous span, so "the <b>end</b>"
                    // doesn't produce a bold space.
                    self.spans.last_mut().unwrap().text.push(' ');
                }
                self.pending_space = false;
                self.push_char(c, style);
            }
        }
    }

    fn push_char(&mut self, c: char, style: Style) {
        match self.spans.last_mut() {
            Some(span) if span.style == style => span.text.push(c),
            _ => self.spans.push(Span { text: c.to_string(), style }),
        }
    }

    fn line_break(&mut self) {
        if self.has_text() {
            self.spans.last_mut().unwrap().text.push('\n');
        }
        self.pending_space = false;
    }

    fn has_text(&self) -> bool {
        self.spans.iter().any(|s| !s.text.is_empty())
    }

    fn flush(&mut self) {
        self.pending_space = false;
        if let Some(last) = self.spans.last_mut() {
            if self.kind != BlockKind::Preformatted {
                let trimmed = last.text.trim_end().len();
                last.text.truncate(trimmed);
            }
        }
        self.spans.retain(|s| !s.text.is_empty());
        if !self.spans.is_empty() {
            let spans = std::mem::take(&mut self.spans);
            self.blocks.push(Block { kind: self.kind, spans });
        }
    }
}

/// XHTML chapters may use HTML named entities like `&nbsp;` or `&mdash;`,
/// which an XML parser doesn't know. Swap the common ones for numeric
/// references and drop any unknown ones rather than failing the whole chapter.
fn replace_html_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let name_end = rest[1..]
            .find(|c: char| !c.is_ascii_alphanumeric())
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let name = &rest[1..name_end];
        let is_named_entity = !name.is_empty() && rest[name_end..].starts_with(';');
        if !is_named_entity || matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        if let Some(code) = html_entity(name) {
            out.push_str(&format!("&#{code};"));
        }
        rest = &rest[name_end + 1..];
    }
    out.push_str(rest);
    out
}

fn html_entity(name: &str) -> Option<u32> {
    Some(match name {
        "nbsp" => 0xa0,
        "ensp" => 0x2002,
        "emsp" => 0x2003,
        "thinsp" => 0x2009,
        "shy" => 0xad,
        "ndash" => 0x2013,
        "mdash" => 0x2014,
        "lsquo" => 0x2018,
        "rsquo" => 0x2019,
        "sbquo" => 0x201a,
        "ldquo" => 0x201c,
        "rdquo" => 0x201d,
        "bdquo" => 0x201e,
        "laquo" => 0xab,
        "raquo" => 0xbb,
        "hellip" => 0x2026,
        "bull" => 0x2022,
        "middot" => 0xb7,
        "dagger" => 0x2020,
        "Dagger" => 0x2021,
        "sect" => 0xa7,
        "para" => 0xb6,
        "deg" => 0xb0,
        "copy" => 0xa9,
        "reg" => 0xae,
        "trade" => 0x2122,
        "times" => 0xd7,
        "pound" => 0xa3,
        "eacute" => 0xe9,
        "egrave" => 0xe8,
        "ecirc" => 0xea,
        "aacute" => 0xe1,
        "agrave" => 0xe0,
        "acirc" => 0xe2,
        "iacute" => 0xed,
        "oacute" => 0xf3,
        "uacute" => 0xfa,
        "ccedil" => 0xe7,
        "ntilde" => 0xf1,
        "auml" => 0xe4,
        "ouml" => 0xf6,
        "uuml" => 0xfc,
        "aelig" => 0xe6,
        "oelig" => 0x153,
        "szlig" => 0xdf,
        _ => return None,
    })
}
