# BookTyper

**One book. One keystroke at a time.**

BookTyper is a free Windows desktop app for typing your way through your own EPUB books. Read at your own pace, enjoy reactive backgrounds, and return to your saved place.

**[Website & interactive demo](https://booktyper.kelptalks.chatgpt.site)** · **[Download for Windows](https://github.com/Kelptalks/BookTyper/releases/latest)** · **[Report an issue](https://github.com/Kelptalks/BookTyper/issues)**

All backgrounds and settings are included. No account, subscription, or paid visual packs.

## Get the app

1. Download `BookTyper-v0.1.0-windows-x64.zip` from [Releases](https://github.com/Kelptalks/BookTyper/releases/latest).
2. Extract the ZIP to a folder on your computer.
3. Open `BookTyper.exe`.
4. Drag a DRM-free `.epub` file onto the app. A short original sample is included in the ZIP's `books` folder if you want to try it first.

The current prebuilt release is for **64-bit Windows**. It is an early release and is not code-signed. Mac, Linux, and mobile do not have tested releases yet.

## Using BookTyper

The library finds EPUB files in your Downloads, Documents, Desktop, and the working directory's `books` folder, plus books you have opened before. Select a book with the arrow keys and Enter, click it, or drag another EPUB onto the window.

Type the words on the page. Use Backspace to correct a mistake. Your position is saved locally, so you can leave and return later. Keep the EPUB in the same location so its saved progress can be matched to it.

Open the gear in the top-right corner to change colors, text feedback, and backgrounds. For the layered web with sparks, choose **Background → Marble**. Explore Dots, Grain, and Streaks, or choose Plain for a quiet page. The effects have adjustable settings.

| Shortcut | Action |
| --- | --- |
| `F11` | Toggle fullscreen |
| `Esc` | Go back; from the library, exit |
| `Tab` | Open or close the chapter menu while reading |
| `Ctrl` + `Left` / `Right` | Previous / next chapter |
| `Ctrl` + `Up` / `Down` | Previous / next paragraph |
| `Backspace` | Correct a character |
| `Ctrl` + `Backspace` | Go back a word |
| `Ctrl` + `+` / `-` | Adjust text size |

## Books and privacy

- **DRM-free EPUB only.** Encrypted ebooks, Kindle-specific formats, PDFs, and plain text files are not supported.
- Bring books you are entitled to use. [Standard Ebooks](https://standardebooks.org/ebooks) is one place to find EPUB editions; check the title's availability in your country.
- The included *First Pages* sample is original text created for this project. No personal book library is distributed with the app or source.
- Books, reading progress, chapter statistics, and settings stay on your computer. The desktop app does not require an account or network connection.
- On Windows, settings and progress live in `%APPDATA%\BookTyper`. Back up that folder to preserve your settings and progress before moving computers.

## Build from source

Install a current [Rust toolchain](https://www.rust-lang.org/tools/install). Windows builds use the MSVC toolchain and require the Visual Studio C++ build tools and Windows SDK.

```powershell
git clone https://github.com/Kelptalks/BookTyper.git
cd BookTyper
cargo run --release
```

To open a particular book:

```powershell
cargo run --release -- "C:\path\to\book.epub"
```

To build or run the existing tests:

```powershell
cargo build --release
cargo test --release
```

The executable is written to `target/release/book_typer.exe`. The app uses Rust, egui/eframe, and wgpu. On Windows it uses Georgia from the system font directory when available, with a fallback font otherwise. No Windows font files are bundled.

The current library discovery and settings paths are Windows-specific. Successful compilation on another operating system does not establish that its user experience or persistence works there.

See [EPUB loader notes](docs/epub-format.md) for details of the book parser.

## Feedback and contributions

Bug reports, ideas, and small contributions are welcome through [GitHub Issues](https://github.com/Kelptalks/BookTyper/issues) and pull requests. For a bug, include your Windows version, what you were doing, and the steps to reproduce it. Please do not attach copyrighted books or private files; use the included sample when possible.

This is a personal project, with no promised release schedule. The goal is a useful, enjoyable app people can actually use.

## License

BookTyper's source code and original sample are available under the [MIT license](LICENSE). Dependencies retain their own licenses.
