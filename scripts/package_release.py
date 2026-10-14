"""Package an already-built Windows release, an original sample, and notices.

Run `cargo build --release`, then `python scripts/package_release.py`.
Personal books and local reading data are never included.
"""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED, ZIP_STORED
from xml.sax.saxutils import escape
import hashlib
import json
import os
import tomllib

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
OUT = ROOT / "release"
OUT.mkdir(exist_ok=True)
EXE = ROOT / "target" / "release" / "book_typer.exe"
if not EXE.is_file():
    raise SystemExit("Run cargo build --release first.")

paragraphs = [
    "Somewhere between the first word and the next page, you find your rhythm.",
    "There is no clock to beat here. The room is quiet, the page is waiting, and the next letter is close enough to touch. You do not have to finish a chapter today. You only have to begin.",
    "Outside, a small breeze moves through the branches. Inside, your fingers find their own weather. A pause, a word, another sentence. The little mistakes are part of it. Go back when you need to, then carry on.",
    "This is a short sample written for BookTyper. To continue with a book of your own, drag a DRM-free EPUB file onto the app. The settings gear lets you change the colors and backgrounds. Try the Marble background for a web of light, or Plain for a quiet page.",
    "One book. One keystroke at a time.",
]
sample = ROOT / "examples" / "first-pages.epub"
with ZipFile(sample, "w", compression=ZIP_DEFLATED) as z:
    z.writestr("mimetype", "application/epub+zip", compress_type=ZIP_STORED)
    z.writestr("META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
    z.writestr("EPUB/package.opf", '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:booktyper:first-pages</dc:identifier><dc:title>First Pages</dc:title><dc:creator>BookTyper</dc:creator><dc:language>en</dc:language><dc:rights>MIT License; copyright 2026 Spencer Frank</dc:rights><meta property="dcterms:modified">2026-10-14T00:00:00Z</meta></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="chapter"/></spine></package>')
    z.writestr("EPUB/chapter.xhtml", '<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>First Pages</title></head><body><h1>First Pages</h1>' + ''.join('<p>' + escape(p) + '</p>' for p in paragraphs) + '</body></html>')
    z.writestr("EPUB/nav.xhtml", '<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><h1>Contents</h1><ol><li><a href="chapter.xhtml">First Pages</a></li></ol></nav></body></html>')

lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
registries = list((cargo_home / "registry" / "src").glob("*"))
notices = ["BookTyper third-party notices", "This distribution uses the following Rust dependencies. Their licenses remain in effect."]
license_files = []
for package in lock["package"]:
    if not package.get("source", "").startswith("registry+"):
        continue
    name = f'{package["name"]}-{package["version"]}'
    folder = next((base / name for base in registries if (base / name).is_dir()), None)
    if folder is None:
        continue
    manifest = tomllib.loads((folder / "Cargo.toml").read_text(encoding="utf-8"))
    info = manifest.get("package", {})
    notices.append(f'{name}: {info.get("license", "See included license files")}\n{info.get("repository", "")}')
    for item in folder.rglob("*"):
        if item.is_file() and len(item.relative_to(folder).parts) <= 3 and item.name.lower().startswith(("license", "licence", "copying", "notice", "ofl", "ufl")):
            license_files.append((item, f'licenses/{name}/{item.relative_to(folder).as_posix()}'))

archive = OUT / f"BookTyper-v{VERSION}-windows-x64.zip"
with ZipFile(archive, "w", compression=ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as z:
    z.write(EXE, "BookTyper.exe")
    z.write(ROOT / "README.md", "README.md")
    z.write(ROOT / "LICENSE", "LICENSE.txt")
    z.write(sample, "books/First Pages.epub")
    z.writestr("START-HERE.txt", "BOOKTYPER\n\n1. Extract this ZIP.\n2. Open BookTyper.exe.\n3. Drag books/First Pages.epub onto the window to try the sample, or open your own DRM-free EPUB.\n4. Type the words on the page. Use the gear for backgrounds and colors.\n\nEverything is free. No account needed.\n\nGuide: https://github.com/Kelptalks/BookTyper#using-booktyper\nFeedback: https://github.com/Kelptalks/BookTyper/issues\n\nEarly release for 64-bit Windows. This executable is not code-signed.\n")
    z.writestr("THIRD-PARTY-NOTICES.txt", "\n\n".join(notices))
    for source, destination in license_files:
        z.write(source, destination)
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
(OUT / "SHA256SUMS.txt").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
print(json.dumps({"archive": str(archive), "bytes": archive.stat().st_size, "sha256": digest, "license_files": len(license_files)}))
