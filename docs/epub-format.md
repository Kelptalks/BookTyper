# EPUB loader notes

Technical background for BookTyper's EPUB parser. For the current desktop app
and usage instructions, see the main README.

## How an EPUB works

An `.epub` is a renamed zip file. Inside:

```
mimetype                    "application/epub+zip"
META-INF/container.xml      says where the package file is
OEBPS/content.opf           the package file (folder/name varies per book)
OEBPS/chapter1.xhtml        the actual text, as XHTML web pages
OEBPS/style.css, images...
```

1. **container.xml** has one job: `<rootfile full-path="OEBPS/content.opf"/>`.
2. **content.opf** has three parts:
   - `<metadata>`: `dc:title`, `dc:creator` (author), language, etc.
   - `<manifest>`: every file in the book, `<item id="c1" href="chapter1.xhtml"/>`.
     hrefs are relative to the .opf's folder.
   - `<spine>`: the reading order, `<itemref idref="c1"/>`, pointing at manifest ids.
3. **Each spine file is an XHTML page**, the same `<h1>`, `<p>`, `<b>`, `<i>`
   tags as a website. That's where the text and formatting live.

`src/epub.rs` follows exactly those steps.

## What the loader gives you

```
Book { title, author, chapters: Vec<Chapter> }
  Chapter { path, blocks: Vec<Block> }      one per spine file; .title() = first heading
    Block { kind, spans: Vec<Span> }        a paragraph-like chunk; starts a new line
      kind: Heading(1..=6) | Paragraph | Quote | ListItem | Preformatted
      Span { text, style: Style { bold, italic } }
```

- Joining a block's spans gives its full text (`block.text()` does that). Spaces
  between words are already in there, and whitespace is collapsed like a
  browser would.
- `\n` inside a span comes from `<br/>`, a forced line break (poems,
  addresses, "CHAPTER I.\nDown the Rabbit-Hole").
- `Preformatted` keeps its whitespace exactly as written.

## Things to know

- **Text is real Unicode.** Books use curly quotes `“ ” ‘ ’`, dashes `— –`,
  `…`, non-breaking spaces (`\u{a0}`) and accented letters. When you
  compare keypresses you'll probably want to accept `"` for `“`, `'` for `’`,
  `-` for `—`, `...` for `…`, and a normal space for `\u{a0}`. Up to you.
- **Spine file ≠ chapter.** Publishers split files however they like. Alice
  is one file per chapter. The Gutenberg Pride and Prejudice is ~10 big files,
  each holding many chapters, and its "title" is an illustration caption.
  Inside a file, the `Heading` blocks are the real chapter boundaries. (The
  book's table of contents, `nav.xhtml` or `toc.ncx`, also lists them, but
  the loader doesn't read it yet.)
- **Bold/italic only come from tags** (`<b> <strong> <i> <em> <cite>`). Some
  books style text through CSS classes (`<span class="bold">`). Those come
  through as plain text.
- The cover, title page and (for Gutenberg) the license show up as chapters
  too. You might want to skip them.
- DRM'd store books (Kindle, Kobo, etc.) are encrypted and won't load.
  Free DRM-free books: gutenberg.org, standardebooks.org.
