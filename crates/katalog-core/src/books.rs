//! Books and comics: `Books/` and `Comics/` next to `Music/`.
//!
//! Books are EPUB and PDF files. Comics are CBZ, CBR, PDF, or a folder of
//! images. Names are read the way people save them:
//!
//!   Books/Mikhail Bulgakov - The Master and Margarita (1967).epub
//!   Books/Lem/Solaris.pdf
//!   Comics/Alan Ford/Alan Ford 012 (1975).cbz
//!   Comics/Corto Maltese/Corto Maltese #3.cbr
//!
//! An EPUB's own title, author and cover win over its name. A PDF's title and
//! author come from `pdfinfo`, and its pages are drawn by `pdftoppm` (both are
//! poppler-utils, which zathura's PDF plugin already needs). The first folder
//! under `Comics/` is the series.
//!
//! Where you are in each book, its bookmarks and highlights (with notes) live
//! in the same database, keyed by the book. A book that moves keeps them when
//! the scan finds the same file name and size somewhere else.

use crate::db;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, UNIX_EPOCH};
use tracing::{debug, info};
use walkdir::WalkDir;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS books (
  id           INTEGER PRIMARY KEY,
  kind         TEXT NOT NULL,              -- book | comic
  format       TEXT NOT NULL,              -- epub | pdf | cbz | cbr | dir
  title        TEXT NOT NULL,
  author       TEXT,
  series       TEXT,
  number       REAL,
  year         INTEGER,
  language     TEXT,
  path         TEXT NOT NULL UNIQUE,
  size         INTEGER NOT NULL,
  mtime        INTEGER NOT NULL,
  pages        INTEGER NOT NULL DEFAULT 0, -- pages, or chapters for an EPUB
  cover_id     INTEGER REFERENCES art(id),
  added_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE IF NOT EXISTS reading (
  book_id      INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
  page         INTEGER NOT NULL DEFAULT 0, -- page, or chapter for an EPUB
  block        INTEGER NOT NULL DEFAULT 0, -- paragraph in the chapter
  progress     REAL NOT NULL DEFAULT 0,    -- 0..1 through the whole book
  opened_at    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS bookmarks (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  page         INTEGER NOT NULL,
  block        INTEGER NOT NULL DEFAULT 0,
  label        TEXT NOT NULL DEFAULT '',
  created_at   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS highlights (
  id           INTEGER PRIMARY KEY,
  book_id      INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  page         INTEGER NOT NULL,           -- page, or chapter for an EPUB
  anchor       TEXT NOT NULL,              -- JSON, see Anchor
  text         TEXT NOT NULL,
  color        TEXT NOT NULL DEFAULT 'ochre',
  note         TEXT NOT NULL DEFAULT '',
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS bookmarks_book ON bookmarks(book_id, page, block);
CREATE INDEX IF NOT EXISTS highlights_book ON highlights(book_id, page);
"#;

pub const BOOK_EXTENSIONS: &[&str] = &["epub", "pdf"];
pub const COMIC_EXTENSIONS: &[&str] = &["cbz", "cbr", "pdf", "epub", "zip"];
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];
pub const COLORS: &[&str] = &["ochre", "pink", "green"];

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    // macOS leaves ._ files and __MACOSX folders in zips.
    !file.starts_with("._")
        && !lower.contains("__macosx/")
        && IMAGE_EXTENSIONS
            .iter()
            .any(|e| file.ends_with(&format!(".{e}")))
}

// ---------------------------------------------------------------- names

/// Compares the way people count: "page 2" before "page 10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

fn year_in(s: &str) -> Option<(usize, i64)> {
    // The last (19xx) or [20xx], or a bare year at the end.
    let b = s.as_bytes();
    let mut found = None;
    for i in 0..b.len().saturating_sub(3) {
        if b[i..i + 4].iter().all(u8::is_ascii_digit)
            && (i == 0 || !b[i - 1].is_ascii_digit())
            && (i + 4 == b.len() || !b[i + 4].is_ascii_digit())
        {
            if let Ok(y) = s[i..i + 4].parse::<i64>() {
                if (1400..=2100).contains(&y) {
                    found = Some((i, y));
                }
            }
        }
    }
    found
}

fn tidy(s: &str) -> String {
    let s: String = s.chars().map(|c| if c == '_' { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.trim_matches(|c: char| c == '-' || c == '.' || c == ',' || c.is_whitespace())
        .trim_end_matches(['(', '['])
        .trim()
        .to_string()
}

/// Title, author and year from a book's file name:
/// "Author - Title (1967)", "Title (1967)", "Title - Author" is not guessed.
pub fn parse_book_name(stem: &str) -> (String, Option<String>, Option<i64>) {
    let mut s = stem.replace('_', " ");
    let mut year = None;
    if let Some((i, y)) = year_in(&s) {
        // Only a year that is in brackets or at the end, not "1984" the title.
        let before = s[..i].trim_end();
        let bracketed = before.ends_with('(') || before.ends_with('[');
        if bracketed || (i > 0 && s[i + 4..].trim().is_empty()) {
            year = Some(y);
            let end = s[i + 4..]
                .find([')', ']'])
                .map(|j| i + 4 + j + 1)
                .unwrap_or(i + 4);
            let start = if bracketed { before.len() - 1 } else { i };
            s = format!("{}{}", &s[..start], &s[end.min(s.len())..]);
        }
    }
    let parts: Vec<&str> = s.splitn(2, " - ").collect();
    let (author, title) = match parts.as_slice() {
        [a, t] if !a.trim().is_empty() && !t.trim().is_empty() => (Some(tidy(a)), tidy(t)),
        _ => (None, tidy(&s)),
    };
    let title = if title.is_empty() { tidy(stem) } else { title };
    (title, author, year)
}

/// Series, issue number and year from a comic's file name, given the series
/// folder it is in (if any): "Alan Ford 012 (1975)", "Corto Maltese #3",
/// "Akira v02".
pub fn parse_comic_name(stem: &str, series: Option<&str>) -> (String, Option<f64>, Option<i64>) {
    let mut s = stem.replace('_', " ");
    let mut year = None;
    if let Some((i, y)) = year_in(&s) {
        let before = s[..i].trim_end();
        if before.ends_with('(') || before.ends_with('[') {
            year = Some(y);
            s = s[..before.len() - 1].to_string();
        }
    }
    // Drop other bracketed tags: "(digital)", "[scan group]".
    let mut clean = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = clean.split_whitespace().collect();
    // The issue number is the last word that is a number, with or without
    // #, v, vol. or no.
    let mut number = None;
    let mut cut = words.len();
    for (i, w) in words.iter().enumerate().rev() {
        let lower = w.to_ascii_lowercase();
        let digits = lower
            .trim_start_matches('#')
            .trim_start_matches("vol.")
            .trim_start_matches("vol")
            .trim_start_matches("no.")
            .trim_start_matches('v');
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
            if let Ok(n) = digits.parse::<f64>() {
                if i > 0 || series.is_some() {
                    number = Some(n);
                    cut = i;
                    // "Alan Ford Vol 3": drop a lone "vol" before the number.
                    if i > 0
                        && matches!(
                            words[i - 1].to_ascii_lowercase().as_str(),
                            "vol" | "vol." | "no" | "no." | "issue" | "#"
                        )
                    {
                        cut = i - 1;
                    }
                    break;
                }
            }
        }
    }
    let title = tidy(&words[..cut].join(" "));
    let title = if title.is_empty() {
        series.map(str::to_string).unwrap_or_else(|| tidy(stem))
    } else {
        title
    };
    (title, number, year)
}

// ---------------------------------------------------------------- EPUB

#[derive(Debug, Clone, Default, Serialize)]
pub struct EpubInfo {
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i64>,
    pub language: Option<String>,
    /// The spine, in reading order: each chapter's file inside the zip and
    /// its name from the table of contents.
    pub chapters: Vec<Chapter>,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub href: String,
    pub title: String,
}

fn zip_open(path: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    zip::ZipArchive::new(f).with_context(|| format!("read {}", path.display()))
}

fn zip_bytes(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>> {
    let real = if z.index_for_name(name).is_some() {
        name.to_string()
    } else {
        // Some books get the case of a name wrong, or escape it.
        let decoded = urlencoding::decode(name)
            .map(|c| c.into_owned())
            .unwrap_or_default();
        let want = name.to_lowercase();
        (0..z.len())
            .filter_map(|i| z.name_for_index(i).map(str::to_string))
            .find(|n| n.to_lowercase() == want || *n == decoded)
            .ok_or_else(|| anyhow!("{name} is not in the book"))?
    };
    let mut f = z.by_name(&real)?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v)?;
    Ok(v)
}

/// XHTML from the wild: HTML entities XML does not know, and the odd stray
/// ampersand. roxmltree wants strict XML.
fn xmlish(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let s = s.trim_start_matches('\u{feff}');
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    // Drop the DOCTYPE: roxmltree refuses external DTDs with entities.
    if let Some(i) = rest.find("<!DOCTYPE") {
        if let Some(j) = rest[i..].find('>') {
            out.push_str(&rest[..i]);
            rest = &rest[i + j + 1..];
        }
    }
    let named: &[(&str, &str)] = &[
        ("nbsp", "\u{a0}"),
        ("shy", "\u{ad}"),
        ("mdash", "—"),
        ("ndash", "–"),
        ("hellip", "…"),
        ("lsquo", "‘"),
        ("rsquo", "’"),
        ("ldquo", "“"),
        ("rdquo", "”"),
        ("laquo", "«"),
        ("raquo", "»"),
        ("copy", "©"),
        ("middot", "·"),
        ("bull", "•"),
        ("eacute", "é"),
        ("thinsp", "\u{2009}"),
        ("emsp", "\u{2003}"),
        ("ensp", "\u{2002}"),
        ("zwnj", "\u{200c}"),
    ];
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail.find(';').filter(|&j| {
            j <= 10
                && tail[..j]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '#')
        });
        match end {
            Some(j) => {
                let name = &tail[..j];
                if matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") || name.starts_with('#') {
                    out.push('&');
                    out.push_str(name);
                    out.push(';');
                } else if let Some((_, v)) = named.iter().find(|(n, _)| *n == name) {
                    out.push_str(v);
                } else {
                    out.push(' ');
                }
                rest = &tail[j + 1..];
            }
            None => {
                out.push_str("&amp;");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

fn dir_of_href(href: &str) -> &str {
    href.rfind('/').map(|i| &href[..=i]).unwrap_or("")
}

/// `base` + a relative link, with `..` taken out.
pub fn join_href(base_dir: &str, rel: &str) -> String {
    let rel = rel.split('#').next().unwrap_or(rel);
    let rel = urlencoding::decode(rel)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| rel.to_string());
    let mut parts: Vec<&str> = if rel.starts_with('/') {
        vec![]
    } else {
        base_dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn text_of(n: roxmltree::Node) -> String {
    let t: String = n
        .descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn epub_info(path: &Path) -> Result<EpubInfo> {
    let mut z = zip_open(path)?;
    let container = xmlish(&zip_bytes(&mut z, "META-INF/container.xml")?);
    let doc = roxmltree::Document::parse(&container)?;
    let opf_path = doc
        .descendants()
        .find(|n| local(n) == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| anyhow!("no rootfile in container.xml"))?
        .to_string();
    let opf = xmlish(&zip_bytes(&mut z, &opf_path)?);
    let doc = roxmltree::Document::parse(&opf)?;
    let base = dir_of_href(&opf_path).to_string();

    let mut info = EpubInfo::default();
    let meta = |name: &str| {
        doc.descendants()
            .find(|n| {
                local(n) == name
                    && n.tag_name()
                        .namespace()
                        .is_some_and(|ns| ns.contains("purl.org/dc"))
            })
            .map(text_of)
            .filter(|s| !s.is_empty())
    };
    info.title = meta("title");
    info.author = meta("creator");
    info.language = meta("language");
    info.year = meta("date").and_then(|d| d.get(..4).and_then(|y| y.parse().ok()));

    // id -> (href, media type, properties)
    let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
    for n in doc.descendants().filter(|n| local(n) == "item") {
        if let (Some(id), Some(href)) = (n.attribute("id"), n.attribute("href")) {
            manifest.insert(
                id.to_string(),
                (
                    join_href(&base, href),
                    n.attribute("media-type").unwrap_or("").to_string(),
                    n.attribute("properties").unwrap_or("").to_string(),
                ),
            );
        }
    }
    info.cover = manifest
        .values()
        .find(|(_, _, p)| p.split_whitespace().any(|p| p == "cover-image"))
        .map(|(h, ..)| h.clone())
        .or_else(|| {
            let id = doc
                .descendants()
                .find(|n| local(n) == "meta" && n.attribute("name") == Some("cover"))
                .and_then(|n| n.attribute("content"))?;
            manifest
                .get(id)
                .filter(|(_, m, _)| m.starts_with("image/"))
                .map(|(h, ..)| h.clone())
        })
        .or_else(|| {
            manifest
                .values()
                .find(|(h, m, _)| m.starts_with("image/") && h.to_lowercase().contains("cover"))
                .map(|(h, ..)| h.clone())
        });

    // Names from the table of contents: EPUB 3's nav, else EPUB 2's NCX.
    let mut names: HashMap<String, String> = HashMap::new();
    let nav = manifest
        .values()
        .find(|(_, _, p)| p.split_whitespace().any(|p| p == "nav"))
        .map(|(h, ..)| h.clone());
    let ncx = manifest
        .values()
        .find(|(_, m, _)| m == "application/x-dtbncx+xml")
        .map(|(h, ..)| h.clone());
    if let Some(nav) = nav {
        if let Ok(raw) = zip_bytes(&mut z, &nav) {
            let s = xmlish(&raw);
            if let Ok(d) = roxmltree::Document::parse(&s) {
                let nav_dir = dir_of_href(&nav).to_string();
                for a in d.descendants().filter(|n| local(n) == "a") {
                    if let Some(h) = a.attribute("href") {
                        let k = join_href(&nav_dir, h);
                        let t = text_of(a);
                        if !t.is_empty() {
                            names.entry(k).or_insert(t);
                        }
                    }
                }
            }
        }
    }
    if names.is_empty() {
        if let Some(ncx) = ncx {
            if let Ok(raw) = zip_bytes(&mut z, &ncx) {
                let s = xmlish(&raw);
                if let Ok(d) = roxmltree::Document::parse(&s) {
                    let ncx_dir = dir_of_href(&ncx).to_string();
                    for p in d.descendants().filter(|n| local(n) == "navPoint") {
                        let label = p
                            .children()
                            .find(|c| local(c) == "navLabel")
                            .map(text_of)
                            .unwrap_or_default();
                        let src = p
                            .children()
                            .find(|c| local(c) == "content")
                            .and_then(|c| c.attribute("src"));
                        if let (Some(src), false) = (src, label.is_empty()) {
                            names.entry(join_href(&ncx_dir, src)).or_insert(label);
                        }
                    }
                }
            }
        }
    }

    for r in doc.descendants().filter(|n| local(n) == "itemref") {
        if r.attribute("linear") == Some("no") {
            continue;
        }
        let Some((href, media, _)) = r.attribute("idref").and_then(|id| manifest.get(id)) else {
            continue;
        };
        if !media.contains("html") {
            continue;
        }
        let title = names.get(href).cloned().unwrap_or_default();
        info.chapters.push(Chapter {
            href: href.clone(),
            title,
        });
    }
    // Chapters with no name in the contents take the one before's, numbered.
    let mut last = String::new();
    let mut n = 0;
    for c in info.chapters.iter_mut() {
        if c.title.is_empty() {
            n += 1;
            c.title = if last.is_empty() {
                format!("Part {n}")
            } else {
                format!("{last} ({n})")
            };
        } else {
            last = c.title.clone();
            n = 0;
        }
    }
    Ok(info)
}

/// One paragraph, heading or picture of a chapter, ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// p, h1, h2, h3, quote, pre, li, img, hr
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
    /// For a picture: its path inside the book.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
}

impl Block {
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

struct Walker {
    blocks: Vec<Block>,
    cur: Option<Block>,
    base: String,
}

impl Walker {
    fn start(&mut self, kind: &str) {
        self.flush();
        self.cur = Some(Block {
            kind: kind.to_string(),
            spans: vec![],
            src: None,
        });
    }
    fn flush(&mut self) {
        if let Some(mut b) = self.cur.take() {
            // Collapse the whitespace across spans and trim the ends.
            let mut out: Vec<Span> = vec![];
            let mut space = true;
            for s in b.spans.drain(..) {
                let mut t = String::new();
                for c in s.text.chars() {
                    if c == '\u{2028}' || (c == '\n' && b.kind == "pre") {
                        // A <br>: a new line, and no space after it.
                        while t.ends_with(' ') {
                            t.pop();
                        }
                        t.push('\n');
                        space = true;
                    } else if c.is_whitespace() && c != '\u{a0}' {
                        if !space {
                            t.push(' ');
                            space = true;
                        }
                    } else {
                        t.push(c);
                        space = false;
                    }
                }
                if t.is_empty() {
                    continue;
                }
                match out.last_mut() {
                    Some(l) if l.italic == s.italic && l.bold == s.bold => l.text.push_str(&t),
                    _ => out.push(Span { text: t, ..s }),
                }
            }
            if let Some(l) = out.last_mut() {
                let trimmed = l.text.trim_end().to_string();
                l.text = trimmed;
                if l.text.is_empty() {
                    out.pop();
                }
            }
            if !out.is_empty() {
                b.spans = out;
                self.blocks.push(b);
            }
        }
    }
    fn push_text(&mut self, t: &str, italic: bool, bold: bool) {
        if self.cur.is_none() {
            if t.trim().is_empty() {
                return;
            }
            self.cur = Some(Block {
                kind: "p".into(),
                spans: vec![],
                src: None,
            });
        }
        if let Some(b) = self.cur.as_mut() {
            b.spans.push(Span {
                text: t.to_string(),
                italic,
                bold,
            });
        }
    }
    fn walk(&mut self, n: roxmltree::Node, italic: bool, bold: bool) {
        for c in n.children() {
            if c.is_text() {
                self.push_text(c.text().unwrap_or(""), italic, bold);
                continue;
            }
            if !c.is_element() {
                continue;
            }
            let tag = local(&c).to_ascii_lowercase();
            match tag.as_str() {
                "script" | "style" | "head" | "title" => {}
                "br" => self.push_text("\u{2028}", italic, bold),
                "img" | "image" => {
                    let src = c.attribute("src").or_else(|| {
                        c.attributes()
                            .find(|a| a.name() == "href")
                            .map(|a| a.value())
                    });
                    if let Some(src) = src {
                        self.flush();
                        self.blocks.push(Block {
                            kind: "img".into(),
                            spans: vec![],
                            src: Some(join_href(&self.base, src)),
                        });
                    }
                }
                "hr" => {
                    self.flush();
                    self.blocks.push(Block {
                        kind: "hr".into(),
                        spans: vec![],
                        src: None,
                    });
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "li" | "pre" | "dt" | "dd"
                | "figcaption" | "caption" | "td" | "th" => {
                    let kind = match tag.as_str() {
                        "h1" => "h1",
                        "h2" => "h2",
                        "h3" | "h4" | "h5" | "h6" => "h3",
                        "li" => "li",
                        "pre" => "pre",
                        _ => "p",
                    };
                    // A paragraph inside a quote stays a quote.
                    let kind = match (&self.cur, kind) {
                        (Some(b), "p") if b.kind == "quote" && b.spans.is_empty() => "quote",
                        _ => kind,
                    };
                    self.start(kind);
                    self.walk(c, italic, bold || kind.starts_with('h'));
                    self.flush();
                }
                "blockquote" => {
                    self.start("quote");
                    self.walk(c, italic, bold);
                    self.flush();
                }
                "div" | "section" | "article" | "body" | "main" | "aside" | "header" | "footer"
                | "nav" | "ul" | "ol" | "dl" | "table" | "tr" | "tbody" | "thead" | "figure"
                | "svg" | "html" => {
                    self.flush();
                    self.walk(c, italic, bold);
                    self.flush();
                }
                "em" | "i" | "cite" | "var" | "dfn" => self.walk(c, true, bold),
                "strong" | "b" => self.walk(c, italic, true),
                _ => self.walk(c, italic, bold),
            }
        }
    }
}

/// A chapter of an EPUB as blocks: paragraphs with their italics and bold,
/// headings, pictures. Styles of the book itself are left out on purpose:
/// the reader sets one font for everything.
pub fn chapter_blocks(xhtml: &[u8], href: &str) -> Result<Vec<Block>> {
    let s = xmlish(xhtml);
    let doc = roxmltree::Document::parse_with_options(
        &s,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )?;
    let body = doc
        .descendants()
        .find(|n| local(n).eq_ignore_ascii_case("body"))
        .unwrap_or_else(|| doc.root_element());
    let mut w = Walker {
        blocks: vec![],
        cur: None,
        base: dir_of_href(href).to_string(),
    };
    w.walk(body, false, false);
    w.flush();
    Ok(w.blocks)
}

pub fn epub_chapter(path: &Path, index: usize) -> Result<(Chapter, Vec<Block>)> {
    let info = epub_info(path)?;
    let ch = info
        .chapters
        .get(index)
        .cloned()
        .ok_or_else(|| anyhow!("no chapter {index}"))?;
    let mut z = zip_open(path)?;
    let raw = zip_bytes(&mut z, &ch.href)?;
    Ok((ch.clone(), chapter_blocks(&raw, &ch.href)?))
}

/// A file inside an EPUB (a picture), with its type.
pub fn epub_resource(path: &Path, name: &str) -> Result<(String, Vec<u8>)> {
    let mut z = zip_open(path)?;
    let data = zip_bytes(&mut z, name)?;
    Ok((mime_of(name).to_string(), data))
}

fn mime_of(name: &str) -> &'static str {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".png") {
        "image/png"
    } else if l.ends_with(".gif") {
        "image/gif"
    } else if l.ends_with(".webp") {
        "image/webp"
    } else if l.ends_with(".svg") {
        "image/svg+xml"
    } else if l.ends_with(".bmp") {
        "image/bmp"
    } else {
        "image/jpeg"
    }
}

// ---------------------------------------------------------------- comics

fn has(tool: &str) -> bool {
    Command::new(tool)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// The pages of a comic, in reading order: names inside the archive, or
/// file names in the folder.
pub fn comic_pages(path: &Path, format: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = match format {
        "dir" => std::fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| is_image_name(n))
            .collect(),
        "cbz" | "zip" => {
            let z = zip_open(path)?;
            (0..z.len())
                .filter_map(|i| z.name_for_index(i).map(str::to_string))
                .filter(|n| is_image_name(n))
                .collect()
        }
        "cbr" => {
            // Many .cbr files are zips with the wrong name.
            if let Ok(z) = zip_open(path) {
                (0..z.len())
                    .filter_map(|i| z.name_for_index(i).map(str::to_string))
                    .filter(|n| is_image_name(n))
                    .collect()
            } else if has("bsdtar") {
                let out = Command::new("bsdtar").arg("-tf").arg(path).output()?;
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::to_string)
                    .filter(|n| is_image_name(n))
                    .collect()
            } else if has("unrar") {
                let out = Command::new("unrar")
                    .args(["lb", "-p-"])
                    .arg(path)
                    .output()?;
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::to_string)
                    .filter(|n| is_image_name(n))
                    .collect()
            } else {
                bail!("a CBR needs bsdtar (libarchive) or unrar")
            }
        }
        _ => bail!("{format} has no image pages"),
    };
    names.sort_by(|a, b| natural_cmp(a, b));
    Ok(names)
}

/// The bytes of one page of a comic.
pub fn comic_page(path: &Path, format: &str, name: &str) -> Result<Vec<u8>> {
    match format {
        "dir" => Ok(std::fs::read(path.join(name))?),
        "cbz" | "zip" => zip_bytes(&mut zip_open(path)?, name),
        "cbr" => {
            if let Ok(mut z) = zip_open(path) {
                return zip_bytes(&mut z, name);
            }
            let out = if has("bsdtar") {
                Command::new("bsdtar")
                    .arg("-xOf")
                    .arg(path)
                    .arg(name)
                    .output()?
            } else {
                Command::new("unrar")
                    .args(["p", "-inul", "-p-"])
                    .arg(path)
                    .arg(name)
                    .output()?
            };
            if !out.status.success() || out.stdout.is_empty() {
                bail!("could not read {name} from {}", path.display());
            }
            Ok(out.stdout)
        }
        _ => bail!("{format} has no image pages"),
    }
}

// ---------------------------------------------------------------- PDF

#[derive(Debug, Default, Clone)]
pub struct PdfInfo {
    pub pages: i64,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i64>,
}

pub fn pdf_info(path: &Path) -> Result<PdfInfo> {
    let out = Command::new("pdfinfo")
        .arg(path)
        .output()
        .context("pdfinfo is not installed (poppler-utils)")?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut i = PdfInfo::default();
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Pages" => i.pages = v.parse().unwrap_or(0),
            "Title" if !v.is_empty() => i.title = Some(v.to_string()),
            "Author" if !v.is_empty() => i.author = Some(v.to_string()),
            "CreationDate" => i.year = year_in(v).map(|(_, y)| y),
            _ => {}
        }
    }
    if i.pages == 0 && !out.status.success() {
        bail!("pdfinfo could not read {}", path.display());
    }
    Ok(i)
}

/// A PDF title is often a file name or "Microsoft Word - draft3.doc".
fn usable_pdf_title(t: &str) -> bool {
    let l = t.to_lowercase();
    t.chars().count() > 1
        && !l.ends_with(".pdf")
        && !l.ends_with(".doc")
        && !l.ends_with(".docx")
        && !l.starts_with("microsoft word")
        && !l.starts_with("untitled")
        && !l.contains(".indd")
        && !l.contains(".qxd")
}

/// One page of a PDF as PNG, `width` pixels wide. Pages count from 0.
pub fn pdf_page(path: &Path, page: i64, width: u32) -> Result<Vec<u8>> {
    let n = (page + 1).to_string();
    // pdftoppm writes to a file named after its last argument (older ones
    // do not take "-" for standard output).
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "katalog-page-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let out = Command::new("pdftoppm")
        .args(["-f", &n, "-l", &n, "-png", "-singlefile", "-scale-to-x"])
        .arg(width.to_string())
        .args(["-scale-to-y", "-1"])
        .arg(path)
        .arg(&root)
        .output()
        .context("pdftoppm is not installed (poppler-utils)")?;
    let file = root.with_extension("png");
    let data = std::fs::read(&file);
    let _ = std::fs::remove_file(&file);
    match data {
        Ok(d) if out.status.success() && !d.is_empty() => Ok(d),
        _ => bail!(
            "pdftoppm: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("no page")
        ),
    }
}

/// A word on a PDF page, its box as fractions of the page (0..1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Words on the same line share a line number, counted down the page.
    pub line: u32,
}

/// The words of one page with where they are, from `pdftotext -bbox-layout`,
/// in reading order. Highlights on a PDF are made of these.
pub fn pdf_words(path: &Path, page: i64) -> Result<Vec<Word>> {
    let n = (page + 1).to_string();
    let out = Command::new("pdftotext")
        .args(["-f", &n, "-l", &n, "-bbox-layout"])
        .arg(path)
        .arg("-")
        .output()
        .context("pdftotext is not installed (poppler-utils)")?;
    parse_bbox(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_bbox(html: &str) -> Result<Vec<Word>> {
    let text = xmlish(html.as_bytes());
    let doc = roxmltree::Document::parse(&text)?;
    let page = doc
        .descendants()
        .find(|n| local(n) == "page")
        .ok_or_else(|| anyhow!("no page"))?;
    let num = |n: &roxmltree::Node, a: &str| -> f32 {
        n.attribute(a).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    };
    let (pw, ph) = (num(&page, "width").max(1.0), num(&page, "height").max(1.0));
    let mut words = vec![];
    let mut line_no = 0;
    for line in page.descendants().filter(|n| local(n) == "line") {
        for w in line.children().filter(|n| local(n) == "word") {
            let (x0, y0, x1, y1) = (
                num(&w, "xMin"),
                num(&w, "yMin"),
                num(&w, "xMax"),
                num(&w, "yMax"),
            );
            words.push(Word {
                text: w.text().unwrap_or("").to_string(),
                x: x0 / pw,
                y: y0 / ph,
                w: (x1 - x0) / pw,
                h: (y1 - y0) / ph,
                line: line_no,
            });
        }
        line_no += 1;
    }
    Ok(words)
}

// ---------------------------------------------------------------- dark pages

/// Turns a page dark the way a reader wants it: white paper becomes the
/// ink-coloured page, black type becomes the paper colour, greys fall between.
/// Strongly coloured pixels (photos, maps, a red title) keep their hue and
/// only lose some brightness, so pictures still read.
pub fn darken(img: &mut image::RgbaImage, page: [u8; 3], ink: [u8; 3]) {
    for p in img.pixels_mut() {
        let [r, g, b, a] = p.0;
        let max = r.max(g).max(b) as i32;
        let min = r.min(g).min(b) as i32;
        let chroma = max - min;
        let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
        if chroma > 48 {
            let k = 0.78;
            p.0 = [
                (r as f32 * k) as u8,
                (g as f32 * k) as u8,
                (b as f32 * k) as u8,
                a,
            ];
            continue;
        }
        // lum 1 (paper) -> page colour, lum 0 (type) -> ink colour.
        let mix = |pc: u8, ic: u8| (ic as f32 + (pc as f32 - ic as f32) * lum).round() as u8;
        p.0 = [
            mix(page[0], ink[0]),
            mix(page[1], ink[1]),
            mix(page[2], ink[2]),
            a,
        ];
    }
}

// ---------------------------------------------------------------- scanning

#[derive(Debug, Default, Clone, Serialize)]
pub struct BooksReport {
    pub books: usize,
    pub comics: usize,
    pub added: usize,
    pub removed: usize,
    pub seconds: f64,
}

/// What the scanner reads from a file, before it goes in the database.
#[derive(Debug, Default, Clone)]
pub struct Found {
    pub title: String,
    pub author: Option<String>,
    pub series: Option<String>,
    pub number: Option<f64>,
    pub year: Option<i64>,
    pub language: Option<String>,
    pub pages: i64,
    pub cover: Option<(String, Vec<u8>)>,
}

fn stat(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let size = if m.is_dir() {
        // A folder of pages: its size is the sum of its images.
        std::fs::read_dir(p)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len() as i64)
                    .sum()
            })
            .unwrap_or(0)
    } else {
        m.len() as i64
    };
    Some((size, mtime))
}

/// A small cover: the first page of a comic or PDF, or an EPUB's cover,
/// scaled to at most 600 pixels tall and stored as JPEG.
fn thumb(data: &[u8]) -> Option<(String, Vec<u8>)> {
    let img = image::load_from_memory(data).ok()?;
    let img = if img.height() > 600 {
        img.resize(10_000, 600, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    img.to_rgb8()
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .ok()?;
    Some(("image/jpeg".into(), out.into_inner()))
}

fn cover_in(dir: &Path) -> Option<(String, Vec<u8>)> {
    for name in ["cover", "folder", "poster"] {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            let p = dir.join(format!("{name}.{ext}"));
            if let Ok(data) = std::fs::read(&p) {
                return thumb(&data);
            }
        }
    }
    None
}

/// Reads one file (or folder of pages) under `Books/` or `Comics/`.
pub fn read_item(path: &Path, format: &str, kind: &str, series: Option<&str>) -> Found {
    let stem = if format == "dir" {
        path.file_name()
    } else {
        path.file_stem()
    }
    .map(|s| s.to_string_lossy().to_string())
    .unwrap_or_default();
    let mut f = Found::default();
    if kind == "comic" {
        let (t, n, y) = parse_comic_name(&stem, series);
        f.title = t;
        f.number = n;
        f.year = y;
        f.series = series.map(str::to_string).or_else(|| Some(f.title.clone()));
    } else {
        let (t, a, y) = parse_book_name(&stem);
        f.title = t;
        f.author = a.or_else(|| {
            // Books/Author/Title.epub: the folder is the author.
            series.map(str::to_string)
        });
        f.year = y;
    }
    match format {
        "epub" => match epub_info(path) {
            Ok(i) => {
                if kind == "book" {
                    if let Some(t) = i.title.filter(|t| !t.trim().is_empty()) {
                        f.title = t;
                    }
                    f.author = i.author.or(f.author);
                }
                f.year = f.year.or(i.year);
                f.language = i.language;
                f.pages = i.chapters.len() as i64;
                if let Some(c) = i.cover {
                    f.cover = epub_resource(path, &c).ok().and_then(|(_, d)| thumb(&d));
                }
            }
            Err(e) => debug!("epub {}: {e:#}", path.display()),
        },
        "pdf" => {
            if let Ok(i) = pdf_info(path) {
                f.pages = i.pages;
                if kind == "book" {
                    if let Some(t) = i.title.filter(|t| usable_pdf_title(t)) {
                        f.title = t;
                    }
                    if f.author.is_none() {
                        f.author = i.author;
                    }
                }
            }
            if f.pages > 0 {
                f.cover = pdf_page(path, 0, 400).ok().and_then(|d| thumb(&d));
            }
        }
        _ => {
            if let Ok(pages) = comic_pages(path, format) {
                f.pages = pages.len() as i64;
                if let Some(first) = pages.first() {
                    f.cover = comic_page(path, format, first).ok().and_then(|d| thumb(&d));
                }
            }
        }
    }
    if f.cover.is_none() {
        if let Some(dir) = path.parent() {
            // A cover.jpg counts only when the book is alone in its folder.
            let alone = std::fs::read_dir(dir)
                .map(|d| {
                    d.filter_map(|e| e.ok())
                        .filter(|e| {
                            let p = e.path();
                            p.is_file()
                                && (BOOK_EXTENSIONS.contains(&ext_of(&p).as_str())
                                    || COMIC_EXTENSIONS.contains(&ext_of(&p).as_str()))
                        })
                        .count()
                        <= 1
                })
                .unwrap_or(false);
            if alone && format != "dir" {
                f.cover = cover_in(dir);
            }
        }
    }
    f
}

/// Everything under a root that is a book or a comic, with its format and
/// the folder right under the root (the series, or the author).
fn items_under(root: &Path, kind: &str) -> Vec<(PathBuf, String, Option<String>)> {
    let exts = if kind == "comic" {
        COMIC_EXTENSIONS
    } else {
        BOOK_EXTENSIONS
    };
    let mut out = vec![];
    let mut image_dirs: HashSet<PathBuf> = HashSet::new();
    for e in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
    {
        let p = e.path();
        if !e.file_type().is_file() {
            continue;
        }
        let ext = ext_of(p);
        if exts.contains(&ext.as_str()) {
            out.push((
                p.to_path_buf(),
                if ext == "zip" { "cbz".into() } else { ext },
                None,
            ));
        } else if kind == "comic" && is_image_name(&p.to_string_lossy()) {
            if let Some(d) = p.parent() {
                if d != root {
                    image_dirs.insert(d.to_path_buf());
                }
            }
        }
    }
    // A folder of pages is an issue when it has a few images, not just a cover.
    for d in image_dirs {
        let n = std::fs::read_dir(&d)
            .map(|r| {
                r.filter_map(|e| e.ok())
                    .filter(|e| is_image_name(&e.file_name().to_string_lossy()))
                    .count()
            })
            .unwrap_or(0);
        if n >= 3 {
            out.push((d, "dir".into(), None));
        }
    }
    for item in out.iter_mut() {
        let rel = item.0.strip_prefix(root).unwrap_or(&item.0);
        let mut comps = rel.components();
        let first = comps
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string());
        // Only a folder counts as a series, not the file itself.
        if comps.next().is_some() {
            item.2 = first;
        }
    }
    out.sort_by(|a, b| natural_cmp(&a.0.to_string_lossy(), &b.0.to_string_lossy()));
    out
}

/// Brings the database in line with `<library>/Books` and `<library>/Comics`.
/// Either folder may be missing.
pub fn scan(conn: &mut Connection, library: &Path) -> Result<BooksReport> {
    scan_with(conn, library, &read_item)
}

pub fn scan_with(
    conn: &mut Connection,
    library: &Path,
    read: &dyn Fn(&Path, &str, &str, Option<&str>) -> Found,
) -> Result<BooksReport> {
    let started = Instant::now();
    let mut r = BooksReport::default();
    let tx = conn.transaction()?;
    let mut seen: HashSet<String> = HashSet::new();
    for (dir, kind) in [("Books", "book"), ("Comics", "comic")] {
        let root = library.join(dir);
        if !root.is_dir() {
            continue;
        }
        for (path, format, folder) in items_under(&root, kind) {
            let key = path.to_string_lossy().to_string();
            seen.insert(key.clone());
            let Some((size, mtime)) = stat(&path) else {
                continue;
            };
            if kind == "comic" {
                r.comics += 1;
            } else {
                r.books += 1;
            }
            let known: Option<(i64, i64, i64)> = tx
                .query_row(
                    "SELECT id, size, mtime FROM books WHERE path = ?1",
                    [&key],
                    |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)),
                )
                .optional()?;
            if known.is_some_and(|(_, s, m)| s == size && m == mtime) {
                continue;
            }
            let f = read(&path, &format, kind, folder.as_deref());
            let cover_id = match &f.cover {
                Some((mime, data)) => Some(db::insert_art(&tx, mime, data, "book")?),
                None => None,
            };
            match known {
                Some((id, ..)) => {
                    tx.execute(
                        "UPDATE books SET size = ?2, mtime = ?3, pages = ?4,
                           cover_id = COALESCE(?5, cover_id) WHERE id = ?1",
                        params![id, size, mtime, f.pages, cover_id],
                    )?;
                }
                None => {
                    // The same file moved: keep its id, so its place, bookmarks
                    // and highlights come with it.
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let moved: Option<i64> = tx
                        .query_row(
                            "SELECT id, path FROM books WHERE size = ?1 AND kind = ?2",
                            params![size, kind],
                            |x| Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?)),
                        )
                        .optional()?
                        .filter(|(_, p)| !Path::new(p).exists() && p.ends_with(&name))
                        .map(|(id, _)| id);
                    if let Some(id) = moved {
                        tx.execute(
                            "UPDATE books SET path = ?2, mtime = ?3 WHERE id = ?1",
                            params![id, key, mtime],
                        )?;
                        continue;
                    }
                    tx.execute(
                        "INSERT INTO books (kind, format, title, author, series, number, year, language,
                           path, size, mtime, pages, cover_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        params![
                            kind, format, f.title, f.author, f.series, f.number, f.year, f.language,
                            key, size, mtime, f.pages, cover_id
                        ],
                    )?;
                    r.added += 1;
                }
            }
        }
    }
    // What is gone from the folders leaves the catalogue. Only books under a
    // folder that exists are pruned, so an unmounted disk loses nothing.
    let roots: Vec<PathBuf> = ["Books", "Comics"]
        .iter()
        .map(|d| library.join(d))
        .filter(|d| d.is_dir())
        .collect();
    let all: Vec<(i64, String)> = {
        let mut st = tx.prepare("SELECT id, path FROM books")?;
        let rows = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, p) in all {
        let under = roots.iter().any(|r| Path::new(&p).starts_with(r));
        if under && !seen.contains(&p) {
            tx.execute("DELETE FROM books WHERE id = ?1", [id])?;
            r.removed += 1;
        }
    }
    tx.commit()?;
    r.seconds = started.elapsed().as_secs_f64();
    if r.added + r.removed > 0 {
        info!(
            "books: {} books, {} comics ({} new, {} gone)",
            r.books, r.comics, r.added, r.removed
        );
    }
    Ok(r)
}

// ---------------------------------------------------------------- reading

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: i64,
    pub kind: String,
    pub format: String,
    pub title: String,
    pub author: Option<String>,
    pub series: Option<String>,
    pub number: Option<f64>,
    pub year: Option<i64>,
    pub language: Option<String>,
    pub path: String,
    pub pages: i64,
    pub cover_id: Option<i64>,
    pub added_at: String,
    /// Where you are: page (or chapter), paragraph, and how far through.
    pub page: i64,
    pub block: i64,
    pub progress: f64,
    pub opened_at: Option<String>,
    pub highlight_count: i64,
    pub bookmark_count: i64,
}

const BOOK_SQL: &str = "SELECT b.id, b.kind, b.format, b.title, b.author, b.series, b.number,
    b.year, b.language, b.path, b.pages, b.cover_id, b.added_at,
    COALESCE(r.page, 0), COALESCE(r.block, 0), COALESCE(r.progress, 0), r.opened_at,
    (SELECT COUNT(*) FROM highlights h WHERE h.book_id = b.id),
    (SELECT COUNT(*) FROM bookmarks k WHERE k.book_id = b.id)
    FROM books b LEFT JOIN reading r ON r.book_id = b.id";

fn book_from(r: &rusqlite::Row) -> rusqlite::Result<Book> {
    Ok(Book {
        id: r.get(0)?,
        kind: r.get(1)?,
        format: r.get(2)?,
        title: r.get(3)?,
        author: r.get(4)?,
        series: r.get(5)?,
        number: r.get(6)?,
        year: r.get(7)?,
        language: r.get(8)?,
        path: r.get(9)?,
        pages: r.get(10)?,
        cover_id: r.get(11)?,
        added_at: r.get(12)?,
        page: r.get(13)?,
        block: r.get(14)?,
        progress: r.get(15)?,
        opened_at: r.get(16)?,
        highlight_count: r.get(17)?,
        bookmark_count: r.get(18)?,
    })
}

/// Books and comics, optionally of one kind and matching `q`: title, author,
/// series, year, or a word in a highlight or a note.
pub fn books(conn: &Connection, kind: Option<&str>, q: Option<&str>) -> Result<Vec<Book>> {
    let like = q.map(|q| format!("%{}%", q.trim())).filter(|q| q != "%%");
    let mut st = conn.prepare(&format!(
        "{BOOK_SQL} WHERE (?1 IS NULL OR b.kind = ?1) AND (?2 IS NULL OR b.title LIKE ?2
           OR b.author LIKE ?2 OR b.series LIKE ?2 OR CAST(b.year AS TEXT) LIKE ?2
           OR EXISTS (SELECT 1 FROM highlights h WHERE h.book_id = b.id
                      AND (h.text LIKE ?2 OR h.note LIKE ?2)))
         ORDER BY CASE WHEN b.kind = 'comic' THEN COALESCE(b.series, b.title) ELSE COALESCE(b.author, b.title) END COLLATE NOCASE,
           b.number, b.title COLLATE NOCASE"
    ))?;
    let rows = st.query_map(params![kind, like], book_from)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn book(conn: &Connection, id: i64) -> Result<Option<Book>> {
    Ok(conn
        .query_row(&format!("{BOOK_SQL} WHERE b.id = ?1"), [id], book_from)
        .optional()?)
}

/// Books opened before, the last one first.
pub fn reading_now(conn: &Connection, limit: usize) -> Result<Vec<Book>> {
    let mut st = conn.prepare(&format!(
        "{BOOK_SQL} WHERE r.opened_at IS NOT NULL ORDER BY r.opened_at DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit as i64], book_from)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn set_position(
    conn: &Connection,
    id: i64,
    page: i64,
    block: i64,
    progress: f64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO reading (book_id, page, block, progress, opened_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(book_id) DO UPDATE SET page = excluded.page, block = excluded.block,
           progress = excluded.progress, opened_at = excluded.opened_at",
        params![id, page, block, progress.clamp(0.0, 1.0), db::now_iso()],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub book_id: i64,
    pub page: i64,
    pub block: i64,
    pub label: String,
    pub created_at: String,
}

pub fn bookmarks(conn: &Connection, book_id: i64) -> Result<Vec<Bookmark>> {
    let mut st = conn.prepare(
        "SELECT id, book_id, page, block, label, created_at FROM bookmarks
         WHERE book_id = ?1 ORDER BY page, block",
    )?;
    let rows = st.query_map([book_id], |r| {
        Ok(Bookmark {
            id: r.get(0)?,
            book_id: r.get(1)?,
            page: r.get(2)?,
            block: r.get(3)?,
            label: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Adds a bookmark, or takes it away if the place already has one: the
/// bookmark key toggles.
pub fn toggle_bookmark(
    conn: &Connection,
    book_id: i64,
    page: i64,
    block: i64,
    label: &str,
) -> Result<Option<i64>> {
    let had: Option<i64> = conn
        .query_row(
            "SELECT id FROM bookmarks WHERE book_id = ?1 AND page = ?2 AND block = ?3",
            params![book_id, page, block],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = had {
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO bookmarks (book_id, page, block, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![book_id, page, block, label, db::now_iso()],
    )?;
    Ok(Some(conn.last_insert_rowid()))
}

pub fn rename_bookmark(conn: &Connection, id: i64, label: &str) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE bookmarks SET label = ?2 WHERE id = ?1",
        params![id, label],
    )? > 0)
}

pub fn delete_bookmark(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])? > 0)
}

/// Where a highlight sits. In an EPUB: from a word in one paragraph to a
/// word in another (both included). On a page: the words' boxes, as
/// fractions of the page, and the word range in `pdf_words` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Anchor {
    Text {
        b0: i64,
        w0: i64,
        b1: i64,
        w1: i64,
    },
    Page {
        first: i64,
        last: i64,
        rects: Vec<[f32; 4]>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    pub id: i64,
    pub book_id: i64,
    pub page: i64,
    pub anchor: Anchor,
    pub text: String,
    pub color: String,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
}

fn highlight_from(r: &rusqlite::Row) -> rusqlite::Result<Highlight> {
    let anchor: String = r.get(3)?;
    Ok(Highlight {
        id: r.get(0)?,
        book_id: r.get(1)?,
        page: r.get(2)?,
        anchor: serde_json::from_str(&anchor).unwrap_or(Anchor::Text {
            b0: 0,
            w0: 0,
            b1: 0,
            w1: 0,
        }),
        text: r.get(4)?,
        color: r.get(5)?,
        note: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

const HIGHLIGHT_SQL: &str =
    "SELECT id, book_id, page, anchor, text, color, note, created_at, updated_at FROM highlights";

pub fn highlights(conn: &Connection, book_id: i64) -> Result<Vec<Highlight>> {
    let mut st = conn.prepare(&format!(
        "{HIGHLIGHT_SQL} WHERE book_id = ?1 ORDER BY page, id"
    ))?;
    let rows = st.query_map([book_id], highlight_from)?;
    let mut v: Vec<Highlight> = rows.collect::<rusqlite::Result<_>>()?;
    v.sort_by_key(|h| match h.anchor {
        Anchor::Text { b0, w0, .. } => (h.page, b0, w0),
        Anchor::Page { first, .. } => (h.page, first, 0),
    });
    Ok(v)
}

/// The latest highlights across every book, with the book's title.
pub fn recent_highlights(conn: &Connection, limit: usize) -> Result<Vec<(Highlight, String)>> {
    let mut st = conn.prepare(
        "SELECT h.id, h.book_id, h.page, h.anchor, h.text, h.color, h.note, h.created_at,
           h.updated_at, b.title FROM highlights h JOIN books b ON b.id = h.book_id
         ORDER BY h.updated_at DESC, h.id DESC LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| Ok((highlight_from(r)?, r.get(9)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn highlight(conn: &Connection, id: i64) -> Result<Option<Highlight>> {
    Ok(conn
        .query_row(
            &format!("{HIGHLIGHT_SQL} WHERE id = ?1"),
            [id],
            highlight_from,
        )
        .optional()?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewHighlight {
    pub page: i64,
    pub anchor: Anchor,
    pub text: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

fn color_or_default(c: Option<&str>) -> String {
    c.filter(|c| COLORS.contains(c))
        .unwrap_or("ochre")
        .to_string()
}

pub fn add_highlight(conn: &Connection, book_id: i64, h: &NewHighlight) -> Result<Highlight> {
    let now = db::now_iso();
    conn.execute(
        "INSERT INTO highlights (book_id, page, anchor, text, color, note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            book_id,
            h.page,
            serde_json::to_string(&h.anchor)?,
            h.text.trim(),
            color_or_default(h.color.as_deref()),
            h.note.as_deref().unwrap_or("").trim(),
            now
        ],
    )?;
    highlight(conn, conn.last_insert_rowid())?.ok_or_else(|| anyhow!("highlight vanished"))
}

/// Changes a highlight's colour or note. `None` leaves it as it is.
pub fn update_highlight(
    conn: &Connection,
    id: i64,
    color: Option<&str>,
    note: Option<&str>,
) -> Result<Option<Highlight>> {
    let Some(old) = highlight(conn, id)? else {
        return Ok(None);
    };
    let color = color
        .map(|c| color_or_default(Some(c)))
        .unwrap_or(old.color);
    let note = note.map(|n| n.trim().to_string()).unwrap_or(old.note);
    conn.execute(
        "UPDATE highlights SET color = ?2, note = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, color, note, db::now_iso()],
    )?;
    highlight(conn, id)
}

pub fn delete_highlight(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.execute("DELETE FROM highlights WHERE id = ?1", [id])? > 0)
}

/// A book's highlights and notes as Markdown, to keep or paste elsewhere.
pub fn highlights_markdown(conn: &Connection, book_id: i64) -> Result<String> {
    let b = book(conn, book_id)?.ok_or_else(|| anyhow!("no book {book_id}"))?;
    let chapters = if b.format == "epub" {
        epub_info(Path::new(&b.path))
            .map(|i| i.chapters)
            .unwrap_or_default()
    } else {
        vec![]
    };
    let mut out = format!("# {}\n", b.title);
    if let Some(a) = &b.author {
        out.push_str(&format!("\n{a}\n"));
    }
    let mut last_page = -1;
    for h in highlights(conn, book_id)? {
        if h.page != last_page {
            last_page = h.page;
            let head = match chapters.get(h.page as usize) {
                Some(c) => c.title.clone(),
                None => format!("Page {}", h.page + 1),
            };
            out.push_str(&format!("\n## {head}\n"));
        }
        out.push_str(&format!("\n> {}\n", h.text.replace('\n', "\n> ")));
        if !h.note.is_empty() {
            out.push_str(&format!("\n{}\n", h.note));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("katalog-books-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    const OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Na Drini ćuprija</dc:title>
    <dc:creator>Ivo Andrić</dc:creator>
    <dc:date>1945-03-01</dc:date>
    <dc:language>hr</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c1" href="text/one.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/two.xhtml" media-type="application/xhtml+xml"/>
    <item id="cov" href="img/cover.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
</package>"#;

    const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="toc"><ol><li><a href="text/one.xhtml">Prvo poglavlje</a></li></ol></nav></body></html>"#;

    const ONE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>x</title><style>p{}</style></head>
<body><h1>I</h1>
<p>Most  je&nbsp;bio <em>star</em> &amp; <b>velik</b>.</p>
<div><p>Drugi<br/>red</p></div>
<blockquote><p>Citat.</p></blockquote>
<p><img src="../img/cover.png" alt=""/></p>
<p>   </p>
</body></html>"#;

    fn epub(dir: &Path) -> PathBuf {
        let p = dir.join("Ivo Andrić - Na Drini ćuprija.epub");
        let cover = png(30, 45);
        write_zip(
            &p,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER.as_bytes()),
                ("OEBPS/content.opf", OPF.as_bytes()),
                ("OEBPS/nav.xhtml", NAV.as_bytes()),
                ("OEBPS/text/one.xhtml", ONE.as_bytes()),
                ("OEBPS/text/two.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Kraj.</p></body></html>"),
                ("OEBPS/img/cover.png", &cover),
            ],
        );
        p
    }

    #[test]
    fn book_names() {
        assert_eq!(
            parse_book_name("Mikhail Bulgakov - The Master and Margarita (1967)"),
            (
                "The Master and Margarita".into(),
                Some("Mikhail Bulgakov".into()),
                Some(1967)
            )
        );
        assert_eq!(parse_book_name("Solaris"), ("Solaris".into(), None, None));
        assert_eq!(parse_book_name("1984"), ("1984".into(), None, None));
        assert_eq!(
            parse_book_name("Dune_[1965]"),
            ("Dune".into(), None, Some(1965))
        );
    }

    #[test]
    fn comic_names() {
        assert_eq!(
            parse_comic_name("Alan Ford 012 (1975)", Some("Alan Ford")),
            ("Alan Ford".into(), Some(12.0), Some(1975))
        );
        assert_eq!(
            parse_comic_name("Corto Maltese #3 (digital) [group]", Some("Corto Maltese")),
            ("Corto Maltese".into(), Some(3.0), None)
        );
        assert_eq!(
            parse_comic_name("Akira v02", None),
            ("Akira".into(), Some(2.0), None)
        );
        assert_eq!(parse_comic_name("Maus", None), ("Maus".into(), None, None));
        assert_eq!(
            parse_comic_name("012", Some("Alan Ford")),
            ("Alan Ford".into(), Some(12.0), None)
        );
    }

    #[test]
    fn pages_sort_the_way_people_count() {
        let mut v = ["p10.jpg", "p2.jpg", "p1.jpg", "P03.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["p1.jpg", "p2.jpg", "P03.jpg", "p10.jpg"]);
    }

    #[test]
    fn hrefs_join() {
        assert_eq!(
            join_href("OEBPS/text/", "../img/a%20b.png#x"),
            "OEBPS/img/a b.png"
        );
        assert_eq!(join_href("", "a.xhtml"), "a.xhtml");
    }

    #[test]
    fn epub_reads_its_metadata_contents_and_chapters() {
        let d = tmp("epub");
        let p = epub(&d);
        let i = epub_info(&p).unwrap();
        assert_eq!(i.title.as_deref(), Some("Na Drini ćuprija"));
        assert_eq!(i.author.as_deref(), Some("Ivo Andrić"));
        assert_eq!(i.year, Some(1945));
        assert_eq!(i.cover.as_deref(), Some("OEBPS/img/cover.png"));
        assert_eq!(i.chapters.len(), 2);
        assert_eq!(i.chapters[0].title, "Prvo poglavlje");
        assert_eq!(i.chapters[1].title, "Prvo poglavlje (1)");

        let (_, blocks) = epub_chapter(&p, 0).unwrap();
        let kinds: Vec<&str> = blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["h1", "p", "p", "quote", "img"]);
        assert_eq!(blocks[1].text(), "Most je\u{a0}bio star & velik.");
        assert!(blocks[1].spans.iter().any(|s| s.italic && s.text == "star"));
        assert!(blocks[1].spans.iter().any(|s| s.bold && s.text == "velik"));
        assert_eq!(blocks[2].text(), "Drugi\nred");
        assert_eq!(blocks[4].src.as_deref(), Some("OEBPS/img/cover.png"));
        let (mime, data) = epub_resource(&p, "OEBPS/img/cover.png").unwrap();
        assert_eq!(mime, "image/png");
        assert!(!data.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn comics_read_their_pages_in_order() {
        let d = tmp("cbz");
        let p = d.join("Alan Ford 012.cbz");
        let page = png(20, 30);
        write_zip(
            &p,
            &[
                ("010.png", &page),
                ("002.png", &page),
                ("__MACOSX/._002.png", b"junk"),
                ("001.png", &page),
                ("info.txt", b"x"),
            ],
        );
        assert_eq!(
            comic_pages(&p, "cbz").unwrap(),
            ["001.png", "002.png", "010.png"]
        );
        assert_eq!(comic_page(&p, "cbz", "002.png").unwrap(), page);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pdf_words_come_as_fractions_of_the_page() {
        let html = r#"<html><body><doc><page width="200.0" height="400.0">
<flow><block><line xMin="10" yMin="20" xMax="90" yMax="30">
<word xMin="10.0" yMin="20.0" xMax="50.0" yMax="30.0">We</word>
<word xMin="60.0" yMin="20.0" xMax="90.0" yMax="30.0">need&amp;</word>
</line><line><word xMin="10" yMin="40" xMax="30" yMax="50">mirrors</word></line></block></flow>
</page></doc></body></html>"#;
        let w = parse_bbox(html).unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].text, "need&");
        assert!((w[0].x - 0.05).abs() < 1e-6 && (w[0].y - 0.05).abs() < 1e-6);
        assert_eq!((w[0].line, w[2].line), (0, 1));
    }

    #[test]
    fn dark_pages_swap_paper_and_ink_and_keep_colour() {
        let mut img = image::RgbaImage::from_pixel(3, 1, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(2, 0, image::Rgba([220, 20, 20, 255]));
        darken(&mut img, [21, 20, 15], [226, 221, 164]);
        assert_eq!(img.get_pixel(0, 0).0, [21, 20, 15, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [226, 221, 164, 255]);
        let red = img.get_pixel(2, 0).0;
        assert!(red[0] > 150 && red[1] < 30);
    }

    #[test]
    fn scan_finds_books_and_comics_and_keeps_marks_when_a_book_moves() {
        let lib = tmp("scan");
        std::fs::create_dir_all(lib.join("Music")).unwrap();
        std::fs::create_dir_all(lib.join("Books/Andrić")).unwrap();
        std::fs::create_dir_all(lib.join("Comics/Alan Ford/Alan Ford 013")).unwrap();
        let book = epub(&lib.join("Books"));
        let page = png(20, 30);
        write_zip(
            &lib.join("Comics/Alan Ford/Alan Ford 012 (1975).cbz"),
            &[("1.png", &page), ("2.png", &page)],
        );
        for n in 1..=3 {
            std::fs::write(
                lib.join(format!("Comics/Alan Ford/Alan Ford 013/{n}.png")),
                &page,
            )
            .unwrap();
        }
        let mut conn = db::open(&lib.join("k.db")).unwrap();
        let r = scan(&mut conn, &lib).unwrap();
        assert_eq!((r.books, r.comics, r.added), (1, 2, 3));

        let all = books(&conn, Some("book"), None).unwrap();
        assert_eq!(all.len(), 1);
        let b = &all[0];
        assert_eq!(b.title, "Na Drini ćuprija");
        assert_eq!(b.author.as_deref(), Some("Ivo Andrić"));
        assert_eq!(b.pages, 2);
        assert!(b.cover_id.is_some());

        let comics = books(&conn, Some("comic"), None).unwrap();
        assert_eq!(comics.len(), 2);
        assert_eq!(comics[0].number, Some(12.0));
        assert_eq!(comics[0].year, Some(1975));
        assert_eq!(comics[0].series.as_deref(), Some("Alan Ford"));
        assert_eq!(comics[1].format, "dir");
        assert_eq!(comics[1].pages, 3);

        // Marks.
        set_position(&conn, b.id, 1, 3, 0.6).unwrap();
        assert_eq!(
            toggle_bookmark(&conn, b.id, 1, 3, "here")
                .unwrap()
                .is_some(),
            true
        );
        let h = add_highlight(
            &conn,
            b.id,
            &NewHighlight {
                page: 0,
                anchor: Anchor::Text {
                    b0: 1,
                    w0: 0,
                    b1: 1,
                    w1: 2,
                },
                text: "Most je bio".into(),
                color: Some("pink".into()),
                note: None,
            },
        )
        .unwrap();
        update_highlight(&conn, h.id, None, Some("the bridge")).unwrap();
        assert_eq!(books(&conn, None, Some("bridge")).unwrap().len(), 1);
        assert_eq!(reading_now(&conn, 5).unwrap()[0].id, b.id);
        let md = highlights_markdown(&conn, b.id).unwrap();
        assert!(
            md.contains("## Prvo poglavlje")
                && md.contains("> Most je bio")
                && md.contains("the bridge")
        );

        // The book moves into the author's folder: same id, same marks.
        let moved = lib.join("Books/Andrić").join(book.file_name().unwrap());
        std::fs::rename(&book, &moved).unwrap();
        let r = scan(&mut conn, &lib).unwrap();
        assert_eq!((r.added, r.removed), (0, 0));
        let again = book_by_path(&conn, &moved).unwrap();
        assert_eq!(again.id, b.id);
        assert_eq!(
            (
                again.page,
                again.block,
                again.highlight_count,
                again.bookmark_count
            ),
            (1, 3, 1, 1)
        );

        // A bookmark toggles off.
        assert_eq!(toggle_bookmark(&conn, b.id, 1, 3, "").unwrap(), None);

        // Gone from the folder, gone from the catalogue.
        std::fs::remove_file(&moved).unwrap();
        let r = scan(&mut conn, &lib).unwrap();
        assert_eq!(r.removed, 1);
        assert!(books(&conn, Some("book"), None).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&lib);
    }

    fn book_by_path(conn: &Connection, p: &Path) -> Option<Book> {
        books(conn, None, None)
            .unwrap()
            .into_iter()
            .find(|b| b.path == p.to_string_lossy())
    }
}
