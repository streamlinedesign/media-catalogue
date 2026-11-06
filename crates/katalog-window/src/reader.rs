//! Read: the home for books and comics, and Katalog's own reader.
//!
//! EPUBs are set again in Literata, one column, the size and spacing you
//! choose, on a dark page or on paper. PDF and comic pages come from katalogd
//! as pictures (dark pages recoloured there). Highlights take a colour and a
//! note; bookmarks are as many as you like. Rotate turns the screen a quarter
//! with xrandr on X11, for a foldable held like a book, or else the page.
//!
//! Keys while reading: j k (or the arrows, space) next and previous, b
//! bookmark, d dark or paper, + and - text size, r rotate, m marks, t
//! contents, 1 2 3 highlight the selection, n a note, Esc back.

use crate::api::Client;
use crate::style::{
    self, alpha, BLACK, BOLD, BOOKS, BOOKS_INK, COMICS, CREAM, MONO, MONO_MEDIUM, PAGE_DARK,
    PAGE_PAPER, SERIF, SERIF_BOLD, TYPE_DARK, TYPE_PAPER,
};
use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Program};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, image, mouse_area, rich_text,
    row, scrollable, span, text, text_editor, text_input, vertical_space, Space,
};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Task, Theme};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

// ---- what katalogd sends -----------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Book {
    pub id: i64,
    pub kind: String,
    pub format: String,
    pub title: String,
    pub author: Option<String>,
    pub series: Option<String>,
    pub number: Option<f64>,
    pub year: Option<i64>,
    pub pages: i64,
    pub cover_id: Option<i64>,
    pub added_at: String,
    pub page: i64,
    pub block: i64,
    pub progress: f64,
    pub opened_at: Option<String>,
    pub highlight_count: i64,
    pub bookmark_count: i64,
}

impl Book {
    pub fn is_comic(&self) -> bool {
        self.kind == "comic"
    }
    /// "Alan Ford 12", "Akira 2", or the title.
    pub fn label(&self) -> String {
        match self.number {
            Some(n) if self.is_comic() => {
                let n = if n.fract() == 0.0 {
                    format!("{}", n as i64)
                } else {
                    format!("{n}")
                };
                format!("{} {n}", self.series.as_deref().unwrap_or(&self.title))
            }
            _ => self.title.clone(),
        }
    }
    /// Who or what it is by, for under a cover.
    pub fn byline(&self) -> String {
        let mut parts = vec![];
        if self.is_comic() {
            if let Some(s) = &self.series {
                if *s != self.label() {
                    parts.push(s.clone());
                }
            }
        } else if let Some(a) = &self.author {
            parts.push(surname(a));
        }
        if let Some(y) = self.year {
            parts.push(y.to_string());
        }
        if self.progress >= 0.995 {
            parts.push("read".into());
        } else if self.progress > 0.0 {
            parts.push(format!("{}%", (self.progress * 100.0).round()));
        } else if self.opened_at.is_none() && self.is_comic() {
            parts.push("new".into());
        }
        parts.join(" · ")
    }
    pub fn matches(&self, q: &str) -> bool {
        if q.is_empty() {
            return true;
        }
        let hit = |s: &Option<String>| s.as_deref().is_some_and(|s| s.to_lowercase().contains(q));
        self.title.to_lowercase().contains(q)
            || hit(&self.author)
            || hit(&self.series)
            || self.year.is_some_and(|y| crate::year_matches(y, q))
    }
}

fn surname(a: &str) -> String {
    a.split([',', '&', ';'])
        .next()
        .unwrap_or(a)
        .split_whitespace()
        .last()
        .unwrap_or(a)
        .to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Chapter {
    pub href: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub page: i64,
    pub block: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct Highlight {
    pub id: i64,
    pub book_id: i64,
    pub page: i64,
    pub anchor: Anchor,
    pub text: String,
    pub color: String,
    pub note: String,
    #[serde(default)]
    pub book: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Home {
    pub reading: Vec<Book>,
    pub highlights: Vec<Highlight>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Full {
    #[serde(flatten)]
    pub book: Book,
    pub chapters: Vec<Chapter>,
    pub bookmarks: Vec<Bookmark>,
    pub highlights: Vec<Highlight>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Span {
    pub text: String,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    pub kind: String,
    #[serde(default)]
    pub spans: Vec<Span>,
    pub src: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChapterOut {
    pub index: usize,
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub line: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Toggled {
    pub id: Option<i64>,
}

/// A decoded page, with its size in pixels.
#[derive(Debug, Clone)]
pub struct Pic {
    pub handle: image::Handle,
    pub w: u32,
    pub h: u32,
}

// ---- preferences --------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    Width,
    Page,
    Two,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub size: f32,
    pub line: f32,
    /// Letters to a line, roughly: the column's width.
    pub measure: f32,
    pub dark: bool,
    /// PDF books: fit the width (and scroll), a whole page, or two pages.
    pub pdf_fit: Fit,
    /// Comics on a wide window: a spread with the cover alone.
    pub spread: bool,
    pub right_to_left: bool,
    /// Rotate turns the whole screen (xrandr) when it can, else the page.
    pub rotate_screen: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            size: 21.0,
            line: 1.65,
            measure: 62.0,
            dark: true,
            pdf_fit: Fit::Width,
            spread: true,
            right_to_left: false,
            rotate_screen: true,
        }
    }
}

fn prefs_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("katalog/reader.json"))
}

impl Prefs {
    fn load() -> Self {
        prefs_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let Some(p) = prefs_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, serde_json::to_vec_pretty(self).unwrap_or_default());
        }
    }
}

// ---- turning the screen ------------------------------------------------------------

/// The screen's turns: upright, then a quarter turn either way.
const TURNS: [&str; 3] = ["normal", "left", "right"];

/// The laptop's own screen as xrandr names it: the primary one, else the
/// first connected.
fn screen_output() -> Option<String> {
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return None;
    }
    let out = std::process::Command::new("xrandr")
        .arg("--query")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = s.lines().filter(|l| l.contains(" connected")).collect();
    let pick = lines
        .iter()
        .find(|l| l.contains(" connected primary"))
        .or_else(|| lines.first())?;
    pick.split_whitespace().next().map(str::to_string)
}

fn turn_screen(output: &str, turn: &str) -> Result<(), String> {
    let st = std::process::Command::new("xrandr")
        .args(["--output", output, "--rotate", turn])
        .status()
        .map_err(|e| format!("xrandr: {e}"))?;
    if !st.success() {
        return Err("xrandr could not turn the screen".into());
    }
    map_touch(output);
    Ok(())
}

/// The screen while it is turned; dropping it turns it back.
#[derive(Debug, Default)]
struct Restore(Option<String>);

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(out) = self.0.take() {
            let _ = turn_screen(&out, "normal");
        }
    }
}

/// xrandr turns the picture but not the touchscreen or the pen: map each of
/// them onto the screen again, so a tap lands where the finger is.
fn map_touch(output: &str) {
    let Ok(out) = std::process::Command::new("xinput")
        .args(["list", "--name-only"])
        .output()
    else {
        return;
    };
    for name in String::from_utf8_lossy(&out.stdout).lines() {
        let n = name.to_lowercase();
        let pointer = ["touch", "stylus", "pen", "wacom", "elan", "finger"]
            .iter()
            .any(|k| n.contains(k));
        if pointer && !n.contains("touchpad") {
            let _ = std::process::Command::new("xinput")
                .args(["map-to-output", name, output])
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

// ---- state ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    None,
    Contents,
    Marks,
}

/// What is selected and not yet highlighted.
#[derive(Debug, Clone, PartialEq)]
pub enum Sel {
    /// In an EPUB chapter: from (block, word) to (block, word).
    Text {
        a: (usize, usize),
        b: (usize, usize),
        done: bool,
    },
    /// On a PDF page: a range of its words.
    Page {
        page: i64,
        first: usize,
        last: usize,
    },
}

/// A word of an EPUB paragraph, as it is drawn and clicked.
#[derive(Debug, Clone)]
pub struct Tok {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
    pub word: usize,
}

pub struct Open {
    pub full: Full,
    /// The page (PDF, comic) or chapter (EPUB) on screen.
    pub page: i64,
    /// EPUB: the paragraph at the top, and the chapter itself.
    pub block: i64,
    pub chapter: Option<ChapterOut>,
    pub toks: Vec<Vec<Tok>>,
    /// Pages and EPUB pictures: (page, dark, thumb).
    pub pics: HashMap<(i64, bool, bool), Pic>,
    pub loading: HashSet<(i64, bool, bool)>,
    pub res: HashMap<String, Pic>,
    pub words: HashMap<i64, Vec<Word>>,
    pub sel: Option<Sel>,
    pub side: Side,
    /// The highlight whose note is open, and the note as typed.
    pub editing: Option<i64>,
    pub note: text_editor::Content,
    pub goto: String,
    pub rel: f32,
    pub saved_at: Instant,
    /// The bar over a comic: shown until you read, back with a click in the middle.
    pub chrome: bool,
    pub flash: Option<String>,
}

pub struct Reader {
    pub books: Vec<Book>,
    pub home: Home,
    pub covers: HashMap<i64, Pic>,
    covers_loading: HashSet<i64>,
    pub open: Option<Open>,
    pub prefs: Prefs,
    pub window: Size,
    pub scale: f32,
    /// How far the screen is turned (an index into TURNS), and which screen.
    pub turn: usize,
    output: Option<Option<String>>,
    /// Page-only rotation in quarter turns, when the screen cannot turn.
    pub page_turn: u8,
    /// Which way Rotate turns: 1 left, 2 right (an index into TURNS).
    dir: usize,
    /// Holds the turned screen, to put it back if the window quits turned.
    restore: Restore,
    pub error: Option<String>,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            books: vec![],
            home: Home::default(),
            covers: HashMap::new(),
            covers_loading: HashSet::new(),
            open: None,
            prefs: Prefs::load(),
            window: Size::new(1440.0, 900.0),
            scale: 1.0,
            turn: 0,
            output: None,
            page_turn: 0,
            dir: 1,
            restore: Restore(None),
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Books(Result<Vec<Book>, String>),
    Home(Result<Home, String>),
    Cover(i64, Result<Vec<u8>, String>),
    Open(i64),
    Opened(Result<Full, String>),
    Close,
    Chapter(Result<ChapterOut, String>),
    Pic((i64, bool, bool), Result<Pic, String>),
    Res(String, Result<Pic, String>),
    Words(i64, Result<Vec<Word>, String>),
    Go(i64),
    GoTo(i64, i64),
    Next,
    Prev,
    GotoInput(String),
    GotoSubmit,
    Scrolled(scrollable::Viewport),
    Saved(Result<(), String>),
    Word(usize, usize),
    PageSelect(i64, Point, Point),
    /// A click or tap on a page: where on it (0..1), and how far across the
    /// window it was (0..1), so a spread turns by the half of the whole view.
    PageClick(i64, Point, f32),
    Mark(&'static str),
    Marked(Result<Highlight, String>, bool),
    OpenNote(i64),
    NoteEdit(text_editor::Action),
    SaveNote,
    Recolor(i64, &'static str),
    Changed(Result<Highlight, String>),
    DeleteHighlight(i64),
    Deleted(i64, Result<(), String>),
    Cancel,
    Copy,
    Export,
    Exported(Result<String, String>),
    Bookmark,
    Bookmarked(Result<Toggled, String>),
    DeleteBookmark(i64),
    Refreshed(Result<Full, String>),
    Side(Side),
    Size(f32),
    Line(f32),
    Measure(f32),
    Dark,
    Fit(Fit),
    Spread,
    Direction,
    RotateScreen,
    Rotate,
    RotateOther,
    /// Up or down a step: scrolls a chapter or a page shown to its width,
    /// turns the page otherwise.
    Step(f32),
    Chrome,
}

const COLORS: [&str; 3] = ["ochre", "pink", "green"];

pub fn mark_color(name: &str) -> Color {
    match name {
        "pink" => style::PINK,
        "green" => style::GREEN,
        _ => style::OCHRE,
    }
}

async fn pic(c: Client, path: String) -> Result<Pic, String> {
    let bytes = c.bytes(&path).await?;
    tokio::task::spawn_blocking(move || {
        let img = ::image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Pic {
            handle: image::Handle::from_rgba(w, h, rgba.into_raw()),
            w,
            h,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The words of a paragraph, each with the space after it, keeping italics and bold.
fn tokens(b: &Block) -> Vec<Tok> {
    let mut out: Vec<Tok> = vec![];
    let mut word = 0;
    let mut glue = false;
    for s in &b.spans {
        let mut cur = String::new();
        for ch in s.text.chars() {
            cur.push(ch);
            if ch.is_whitespace() {
                if cur.trim().is_empty() {
                    // Leading space of a span: hang it on the word before.
                    if let Some(l) = out.last_mut() {
                        l.text.push_str(&cur);
                        cur.clear();
                        glue = false;
                        continue;
                    }
                }
                let w = if glue { word - 1 } else { word };
                out.push(Tok {
                    text: std::mem::take(&mut cur),
                    italic: s.italic,
                    bold: s.bold,
                    word: w,
                });
                if !glue {
                    word += 1;
                }
                glue = false;
            }
        }
        if !cur.is_empty() {
            // A word that goes on in the next span ("K" + "."), one word.
            let w = if glue { word - 1 } else { word };
            out.push(Tok {
                text: cur,
                italic: s.italic,
                bold: s.bold,
                word: w,
            });
            if !glue {
                word += 1;
            }
            glue = true;
        }
    }
    out
}

fn ord(a: (usize, usize), b: (usize, usize)) -> ((usize, usize), (usize, usize)) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn in_range(p: (usize, usize), a: (usize, usize), b: (usize, usize)) -> bool {
    p >= a && p <= b
}

/// The word nearest a point on a page (fractions of the page).
fn word_at(words: &[Word], p: Point) -> Option<usize> {
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let dx = if p.x < w.x {
                w.x - p.x
            } else if p.x > w.x + w.w {
                p.x - w.x - w.w
            } else {
                0.0
            };
            let dy = if p.y < w.y {
                w.y - p.y
            } else if p.y > w.y + w.h {
                p.y - w.y - w.h
            } else {
                0.0
            };
            // A line away counts for more than a word away.
            (i, dx + dy * 4.0)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, d)| *d < 0.05)
        .map(|(i, _)| i)
}

/// One box per line for words first..=last.
fn rects(words: &[Word], first: usize, last: usize) -> Vec<[f32; 4]> {
    let mut out: Vec<(u32, [f32; 4])> = vec![];
    for w in words.iter().take(last + 1).skip(first) {
        match out.last_mut() {
            Some((line, r)) if *line == w.line => {
                let x1 = (r[0] + r[2]).max(w.x + w.w);
                let y1 = (r[1] + r[3]).max(w.y + w.h);
                r[0] = r[0].min(w.x);
                r[1] = r[1].min(w.y);
                r[2] = x1 - r[0];
                r[3] = y1 - r[1];
            }
            _ => out.push((w.line, [w.x, w.y, w.w, w.h])),
        }
    }
    out.into_iter().map(|(_, r)| r).collect()
}

impl Reader {
    pub fn load(&self, c: &Client) -> Task<Msg> {
        let a = c.clone();
        let b = c.clone();
        Task::batch([
            Task::perform(async move { a.get("/api/books").await }, Msg::Books),
            Task::perform(async move { b.get("/api/books/home").await }, Msg::Home),
        ])
    }

    fn want_cover(&mut self, c: &Client, id: i64) -> Task<Msg> {
        if self.covers.contains_key(&id) || !self.covers_loading.insert(id) {
            return Task::none();
        }
        let c = c.clone();
        Task::perform(
            async move {
                c.bytes(&format!("/art/{id}?size={}", crate::video::POSTER))
                    .await
            },
            move |r| Msg::Cover(id, r),
        )
    }

    pub fn book(&self, id: i64) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    /// The width to ask katalogd for a page at: the window's, in real pixels.
    fn page_px(&self, two: bool) -> u32 {
        let w = if two {
            self.window.width / 2.0
        } else {
            self.window.width.min(1600.0)
        };
        ((w * self.scale).round() as u32).clamp(400, 3200) / 100 * 100
    }

    fn dark_pages(&self, o: &Open) -> bool {
        self.prefs.dark && !o.full.book.is_comic()
    }

    fn want_pic(&mut self, c: &Client, page: i64, thumb: bool) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        if page < 0 || page >= o.full.book.pages {
            return Task::none();
        }
        let dark = !thumb && self.dark_pages(o);
        let key = (page, dark, thumb);
        if o.pics.contains_key(&key) || o.loading.contains(&key) {
            return Task::none();
        }
        let two = o.full.book.is_comic() && self.spread_on() || self.prefs.pdf_fit == Fit::Two;
        let w = if thumb { 200 } else { self.page_px(two) };
        let id = o.full.book.id;
        let path = format!("/books/{id}/page/{page}?w={w}&dark={}", dark as u8);
        if let Some(o) = self.open.as_mut() {
            o.loading.insert(key);
        }
        let c = c.clone();
        Task::perform(pic(c, path), move |r| Msg::Pic(key, r))
    }

    fn want_words(&mut self, c: &Client, page: i64) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        if o.full.book.format != "pdf" || o.words.contains_key(&page) {
            return Task::none();
        }
        let id = o.full.book.id;
        let c = c.clone();
        Task::perform(
            async move { c.get(&format!("/api/books/{id}/page/{page}/words")).await },
            move |r| Msg::Words(page, r),
        )
    }

    /// Spreads only when the window is wider than it is tall.
    pub fn spread_on(&self) -> bool {
        self.prefs.spread && self.window.width > self.window.height * 1.1
    }

    /// The pages on screen for a comic or PDF: one, or a spread.
    fn shown_pages(&self, o: &Open) -> Vec<i64> {
        let comic = o.full.book.is_comic();
        let two = if comic {
            self.spread_on()
        } else {
            self.prefs.pdf_fit == Fit::Two
        };
        if !two {
            return vec![o.page];
        }
        // The cover stands alone; after it, pages go in pairs 1-2, 3-4.
        if comic && o.page == 0 {
            return vec![0];
        }
        let first = if comic {
            o.page - (o.page - 1) % 2
        } else {
            o.page - o.page % 2
        };
        let mut v = vec![first];
        if first + 1 < o.full.book.pages {
            v.push(first + 1);
        }
        if comic && self.prefs.right_to_left {
            v.reverse();
        }
        v
    }

    /// Asks for what is on screen and what comes next.
    fn fetch_around(&mut self, c: &Client) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        if o.full.book.format == "epub" {
            return Task::none();
        }
        let shown = self.shown_pages(o);
        let last = *shown.iter().max().unwrap_or(&o.page);
        let first = *shown.iter().min().unwrap_or(&o.page);
        let comic = o.full.book.is_comic();
        let mut t = vec![];
        // A comic asks for the small pages it shows first: they come quickly
        // and stand in until the big ones are drawn.
        if comic {
            for &p in &shown {
                t.push(self.want_pic(c, p, true));
            }
        }
        for p in shown.iter().copied().chain([last + 1, last + 2, first - 1]) {
            t.push(self.want_pic(c, p, false));
            t.push(self.want_words(c, p));
        }
        if comic {
            let page = self.open.as_ref().map(|o| o.page).unwrap_or(0);
            for p in (page - 6)..=(page + 8) {
                t.push(self.want_pic(c, p, true));
            }
        }
        Task::batch(t)
    }

    fn load_chapter(&mut self, c: &Client) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        let (id, n) = (o.full.book.id, o.page);
        let c = c.clone();
        Task::perform(
            async move { c.get(&format!("/api/books/{id}/chapters/{n}")).await },
            Msg::Chapter,
        )
    }

    fn progress(&self) -> f64 {
        let Some(o) = self.open.as_ref() else {
            return 0.0;
        };
        let n = o.full.book.pages.max(1) as f64;
        if o.full.book.format == "epub" {
            ((o.page as f64 + o.rel as f64) / n).clamp(0.0, 1.0)
        } else {
            let last = self.shown_pages(o).into_iter().max().unwrap_or(o.page);
            ((last + 1) as f64 / n).clamp(0.0, 1.0)
        }
    }

    fn save_position(&mut self, c: &Client) -> Task<Msg> {
        let progress = self.progress();
        let Some(o) = self.open.as_mut() else {
            return Task::none();
        };
        o.saved_at = Instant::now();
        o.full.book.progress = progress;
        o.full.book.page = o.page;
        o.full.book.block = o.block;
        let (id, page, block) = (o.full.book.id, o.page, o.block);
        if let Some(b) = self.books.iter_mut().find(|b| b.id == id) {
            b.progress = progress;
            b.page = page;
            b.block = block;
            b.opened_at = Some("now".into());
        }
        let c = c.clone();
        Task::perform(
            async move {
                c.send_ok(
                    reqwest::Method::PUT,
                    &format!("/api/books/{id}/position"),
                    Some(serde_json::json!({ "page": page, "block": block, "progress": progress })),
                )
                .await
            },
            Msg::Saved,
        )
    }

    fn turn_page(&mut self, c: &Client, step: i64) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        let pages = o.full.book.pages;
        if o.full.book.format == "epub" {
            let next = (o.page + step).clamp(0, pages - 1);
            if next == o.page {
                return Task::none();
            }
            return self.go(c, next, if step < 0 { -1 } else { 0 });
        }
        let shown = self.shown_pages(o);
        let (lo, hi) = (
            *shown.iter().min().unwrap_or(&o.page),
            *shown.iter().max().unwrap_or(&o.page),
        );
        let next = if step > 0 { hi + 1 } else { lo - 1 };
        if next < 0 || next >= pages {
            return Task::none();
        }
        self.go(c, next, 0)
    }

    /// Goes to a page or chapter. For a chapter, `block` is the paragraph to
    /// show at the top; -1 is the end (going back from the next chapter).
    fn go(&mut self, c: &Client, page: i64, block: i64) -> Task<Msg> {
        let Some(o) = self.open.as_mut() else {
            return Task::none();
        };
        let page = page.clamp(0, (o.full.book.pages - 1).max(0));
        let same = page == o.page && o.chapter.is_some();
        o.page = page;
        o.block = block;
        o.sel = None;
        let epub = o.full.book.format == "epub";
        let save = self.save_position(c);
        if epub {
            if same {
                return Task::batch([save, self.scroll_to_block()]);
            }
            if let Some(o) = self.open.as_mut() {
                o.chapter = None;
                o.toks.clear();
            }
            return Task::batch([save, self.load_chapter(c)]);
        }
        let top = scrollable::snap_to(
            scrollable::Id::new("page"),
            scrollable::RelativeOffset::START,
        );
        Task::batch([save, top, self.fetch_around(c)])
    }

    fn scroll_to_block(&self) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        let n = o
            .chapter
            .as_ref()
            .map(|c| c.blocks.len())
            .unwrap_or(0)
            .max(1);
        let y = if o.block < 0 {
            1.0
        } else {
            (o.block as f32 / n as f32).clamp(0.0, 1.0)
        };
        scrollable::snap_to(
            scrollable::Id::new("chapter"),
            scrollable::RelativeOffset { x: 0.0, y },
        )
    }

    fn refresh(&self, c: &Client) -> Task<Msg> {
        let Some(o) = self.open.as_ref() else {
            return Task::none();
        };
        let id = o.full.book.id;
        let c = c.clone();
        Task::perform(
            async move { c.get(&format!("/api/books/{id}")).await },
            Msg::Refreshed,
        )
    }

    fn selection_text(&self) -> Option<(i64, Anchor, String)> {
        let o = self.open.as_ref()?;
        match o.sel.as_ref()? {
            Sel::Text { a, b, .. } => {
                let (a, b) = ord(*a, *b);
                let mut words = String::new();
                for (bi, toks) in o.toks.iter().enumerate().take(b.0 + 1).skip(a.0) {
                    for t in toks {
                        if in_range((bi, t.word), a, b) {
                            words.push_str(&t.text);
                        }
                    }
                    if bi < b.0 {
                        words.push('\n');
                    }
                }
                Some((
                    o.page,
                    Anchor::Text {
                        b0: a.0 as i64,
                        w0: a.1 as i64,
                        b1: b.0 as i64,
                        w1: b.1 as i64,
                    },
                    words.trim().to_string(),
                ))
            }
            Sel::Page { page, first, last } => {
                let words = o.words.get(page)?;
                let text = words
                    .iter()
                    .take(last + 1)
                    .skip(*first)
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                Some((
                    *page,
                    Anchor::Page {
                        first: *first as i64,
                        last: *last as i64,
                        rects: rects(words, *first, *last),
                    },
                    text,
                ))
            }
        }
    }

    pub fn close(&mut self, c: &Client) -> Task<Msg> {
        let save = if self.open.is_some() {
            self.save_position(c)
        } else {
            Task::none()
        };
        self.open = None;
        self.unturn();
        Task::batch([save, self.load(c)])
    }

    /// Turns the screen to TURNS[to], or only the page when the screen cannot turn.
    fn turn_to(&mut self, to: usize) -> Task<Msg> {
        if self.output.is_none() {
            self.output = Some(screen_output());
        }
        match (&self.output, self.prefs.rotate_screen) {
            (Some(Some(out)), true) => match turn_screen(out, TURNS[to]) {
                Ok(()) => {
                    self.turn = to;
                    self.restore.0 = (to != 0).then(|| out.clone());
                }
                Err(e) => self.error = Some(e),
            },
            _ => {
                let epub = self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.full.book.format == "epub");
                if epub {
                    if let Some(o) = self.open.as_mut() {
                        o.flash =
                            Some("turning an EPUB needs the screen to turn: xrandr, on X11".into());
                    }
                } else {
                    // Left is a quarter turn back, right a quarter turn on.
                    self.page_turn = [0, 3, 1][to];
                }
            }
        }
        Task::none()
    }

    /// Puts the screen back the right way up.
    pub fn unturn(&mut self) {
        if self.turn != 0 {
            if let Some(Some(out)) = &self.output {
                let _ = turn_screen(out, "normal");
            }
            self.turn = 0;
            self.restore.0 = None;
        }
        self.page_turn = 0;
    }

    pub fn update(&mut self, m: Msg, c: &Client) -> Task<Msg> {
        match m {
            Msg::Books(Ok(b)) => {
                self.books = b;
                self.error = None;
                let ids: Vec<i64> = self.books.iter().filter_map(|b| b.cover_id).collect();
                Task::batch(ids.into_iter().map(|id| self.want_cover(c, id)))
            }
            Msg::Books(Err(e)) | Msg::Home(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Msg::Home(Ok(h)) => {
                self.home = h;
                let ids: Vec<i64> = self
                    .home
                    .reading
                    .iter()
                    .filter_map(|b| b.cover_id)
                    .collect();
                Task::batch(ids.into_iter().map(|id| self.want_cover(c, id)))
            }
            Msg::Cover(id, r) => {
                self.covers_loading.remove(&id);
                if let Ok(bytes) = r {
                    if let Ok(img) = ::image::load_from_memory(&bytes) {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        self.covers.insert(
                            id,
                            Pic {
                                handle: image::Handle::from_rgba(w, h, rgba.into_raw()),
                                w,
                                h,
                            },
                        );
                    }
                }
                Task::none()
            }
            Msg::Open(id) => {
                if self.open.as_ref().is_some_and(|o| o.full.book.id == id) {
                    return Task::none();
                }
                let save = if self.open.is_some() {
                    self.save_position(c)
                } else {
                    Task::none()
                };
                let c = c.clone();
                Task::batch([
                    save,
                    Task::perform(
                        async move { c.get(&format!("/api/books/{id}")).await },
                        Msg::Opened,
                    ),
                ])
            }
            Msg::Opened(Ok(full)) | Msg::Refreshed(Ok(full))
                if self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.full.book.id == full.book.id) =>
            {
                if let Some(o) = self.open.as_mut() {
                    o.full.bookmarks = full.bookmarks;
                    o.full.highlights = full.highlights;
                }
                Task::none()
            }
            Msg::Opened(Ok(full)) => {
                let (page, block) = (full.book.page, full.book.block);
                self.open = Some(Open {
                    page,
                    block,
                    chapter: None,
                    toks: vec![],
                    pics: HashMap::new(),
                    loading: HashSet::new(),
                    res: HashMap::new(),
                    words: HashMap::new(),
                    sel: None,
                    side: Side::None,
                    editing: None,
                    note: text_editor::Content::new(),
                    goto: String::new(),
                    rel: 0.0,
                    saved_at: Instant::now(),
                    chrome: true,
                    flash: None,
                    full,
                });
                self.error = None;
                let epub = self
                    .open
                    .as_ref()
                    .is_some_and(|o| o.full.book.format == "epub");
                if epub {
                    Task::batch([self.load_chapter(c), self.save_position(c)])
                } else {
                    Task::batch([self.fetch_around(c), self.save_position(c)])
                }
            }
            Msg::Refreshed(Ok(_)) => Task::none(),
            Msg::Opened(Err(e)) | Msg::Refreshed(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Msg::Close => self.close(c),
            Msg::Chapter(Ok(ch)) => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                if ch.index as i64 != o.page {
                    return Task::none();
                }
                o.toks = ch.blocks.iter().map(tokens).collect();
                let pics: Vec<String> = ch.blocks.iter().filter_map(|b| b.src.clone()).collect();
                if o.block < 0 {
                    o.block = ch.blocks.len() as i64 - 1;
                }
                o.chapter = Some(ch);
                let id = o.full.book.id;
                let mut t = vec![self.scroll_to_block()];
                for src in pics {
                    let c = c.clone();
                    let path = format!("/books/{id}/res?path={}", crate::urlencode(&src));
                    t.push(Task::perform(pic(c, path), move |r| {
                        Msg::Res(src.clone(), r)
                    }));
                }
                Task::batch(t)
            }
            Msg::Chapter(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Msg::Pic(key, r) => {
                if let Some(o) = self.open.as_mut() {
                    o.loading.remove(&key);
                    match r {
                        Ok(p) => {
                            o.pics.insert(key, p);
                        }
                        Err(e) if !key.2 => o.flash = Some(e),
                        Err(_) => {}
                    }
                }
                Task::none()
            }
            Msg::Res(src, Ok(p)) => {
                if let Some(o) = self.open.as_mut() {
                    o.res.insert(src, p);
                }
                Task::none()
            }
            Msg::Res(_, Err(_)) => Task::none(),
            Msg::Words(page, r) => {
                if let Some(o) = self.open.as_mut() {
                    o.words.insert(page, r.unwrap_or_default());
                }
                Task::none()
            }
            Msg::Go(p) => self.go(c, p, 0),
            Msg::GoTo(p, b) => self.go(c, p, b),
            Msg::Next => self.turn_page(c, 1),
            Msg::Prev => self.turn_page(c, -1),
            Msg::GotoInput(s) => {
                if let Some(o) = self.open.as_mut() {
                    o.goto = s.chars().filter(char::is_ascii_digit).collect();
                }
                Task::none()
            }
            Msg::GotoSubmit => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                let n: i64 = o.goto.parse().unwrap_or(0);
                o.goto.clear();
                if n < 1 {
                    return Task::none();
                }
                self.go(c, n - 1, 0)
            }
            Msg::Scrolled(v) => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                if o.full.book.format != "epub" {
                    return Task::none();
                }
                let rel = v.relative_offset().y;
                let rel = if rel.is_nan() { 0.0 } else { rel };
                o.rel = rel;
                let n = o.toks.len().max(1);
                o.block = ((rel * n as f32).floor() as i64).min(n as i64 - 1).max(0);
                if o.saved_at.elapsed().as_secs() >= 4 {
                    return self.save_position(c);
                }
                Task::none()
            }
            Msg::Saved(Ok(())) => Task::none(),
            Msg::Saved(Err(e)) => {
                if let Some(o) = self.open.as_mut() {
                    o.flash = Some(format!("not saved: {e}"));
                }
                Task::none()
            }
            Msg::Word(b, w) => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                // A click on a highlight with nothing selected opens it.
                if o.sel.is_none() {
                    let page = o.page;
                    let hit = o.full.highlights.iter().find(|h| {
                        h.page == page
                            && match h.anchor {
                                Anchor::Text { b0, w0, b1, w1 } => in_range(
                                    (b, w),
                                    (b0 as usize, w0 as usize),
                                    (b1 as usize, w1 as usize),
                                ),
                                _ => false,
                            }
                    });
                    if let Some(h) = hit {
                        let id = h.id;
                        return self.update(Msg::OpenNote(id), c);
                    }
                }
                o.sel = match o.sel.take() {
                    Some(Sel::Text { a, done: false, .. }) => Some(Sel::Text {
                        a,
                        b: (b, w),
                        done: true,
                    }),
                    _ => Some(Sel::Text {
                        a: (b, w),
                        b: (b, w),
                        done: false,
                    }),
                };
                Task::none()
            }
            Msg::PageSelect(page, a, b) => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                let Some(words) = o.words.get(&page) else {
                    return Task::none();
                };
                if let (Some(x), Some(y)) = (word_at(words, a), word_at(words, b)) {
                    o.sel = Some(Sel::Page {
                        page,
                        first: x.min(y),
                        last: x.max(y),
                    });
                }
                Task::none()
            }
            Msg::PageClick(page, p, across) => {
                let side_w =
                    self.open.as_ref().map_or(
                        0.0,
                        |o| {
                            if o.side != Side::None {
                                SIDE_W
                            } else {
                                0.0
                            }
                        },
                    );
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                // Which half of the view: the left half goes back, the right on
                // (the other way round for right to left).
                let x = across * self.window.width.max(1.0);
                let mid = side_w + (self.window.width - side_w) / 2.0;
                let on = (x >= mid) != (o.full.book.is_comic() && self.prefs.right_to_left);
                if o.full.book.is_comic() {
                    // The top edge of a comic page shows or hides the bar.
                    if p.y < 0.1 {
                        o.chrome = !o.chrome;
                        return Task::none();
                    }
                    return self.turn_page(c, if on { 1 } else { -1 });
                }
                let had_sel = o.sel.take().is_some();
                let hit = o.full.highlights.iter().find(|h| {
                    h.page == page
                        && match &h.anchor {
                            Anchor::Page { rects, .. } => rects.iter().any(|r| {
                                p.x >= r[0]
                                    && p.x <= r[0] + r[2]
                                    && p.y >= r[1] - 0.004
                                    && p.y <= r[1] + r[3] + 0.004
                            }),
                            _ => false,
                        }
                });
                if let Some(h) = hit {
                    let id = h.id;
                    return self.update(Msg::OpenNote(id), c);
                }
                // A tap that only clears a selection does not turn the page.
                if had_sel {
                    return Task::none();
                }
                self.turn_page(c, if on { 1 } else { -1 })
            }
            Msg::Mark(color) => {
                let Some((page, anchor, text)) = self.selection_text() else {
                    return Task::none();
                };
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                o.sel = None;
                let with_note = color == "note";
                let color = if with_note { "ochre" } else { color };
                let id = o.full.book.id;
                let c = c.clone();
                Task::perform(
                    async move {
                        c.post(
                            &format!("/api/books/{id}/highlights"),
                            Some(serde_json::json!({ "page": page, "anchor": anchor, "text": text, "color": color })),
                        )
                        .await
                    },
                    move |r| Msg::Marked(r, with_note),
                )
            }
            Msg::Marked(Ok(h), with_note) => {
                let id = h.id;
                if let Some(o) = self.open.as_mut() {
                    o.full.highlights.push(h);
                }
                if with_note {
                    return self.update(Msg::OpenNote(id), c);
                }
                Task::none()
            }
            Msg::Marked(Err(e), _)
            | Msg::Changed(Err(e))
            | Msg::Bookmarked(Err(e))
            | Msg::Exported(Err(e)) => {
                if let Some(o) = self.open.as_mut() {
                    o.flash = Some(e);
                }
                Task::none()
            }
            Msg::OpenNote(id) => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                let note = o
                    .full
                    .highlights
                    .iter()
                    .find(|h| h.id == id)
                    .map(|h| h.note.clone())
                    .unwrap_or_default();
                o.editing = Some(id);
                o.note = text_editor::Content::with_text(&note);
                o.side = Side::Marks;
                Task::none()
            }
            Msg::NoteEdit(a) => {
                if let Some(o) = self.open.as_mut() {
                    o.note.perform(a);
                }
                Task::none()
            }
            Msg::SaveNote => {
                let Some(o) = self.open.as_mut() else {
                    return Task::none();
                };
                let Some(id) = o.editing.take() else {
                    return Task::none();
                };
                let note = o.note.text().trim().to_string();
                let c = c.clone();
                Task::perform(
                    async move {
                        c.put(
                            &format!("/api/highlights/{id}"),
                            serde_json::json!({ "note": note }),
                        )
                        .await
                    },
                    Msg::Changed,
                )
            }
            Msg::Recolor(id, color) => {
                let c = c.clone();
                Task::perform(
                    async move {
                        c.put(
                            &format!("/api/highlights/{id}"),
                            serde_json::json!({ "color": color }),
                        )
                        .await
                    },
                    Msg::Changed,
                )
            }
            Msg::Changed(Ok(h)) => {
                if let Some(o) = self.open.as_mut() {
                    if let Some(old) = o.full.highlights.iter_mut().find(|x| x.id == h.id) {
                        *old = h;
                    }
                }
                Task::none()
            }
            Msg::DeleteHighlight(id) => {
                let c = c.clone();
                Task::perform(
                    async move {
                        c.send_ok(
                            reqwest::Method::DELETE,
                            &format!("/api/highlights/{id}"),
                            None,
                        )
                        .await
                    },
                    move |r| Msg::Deleted(id, r),
                )
            }
            Msg::Deleted(id, r) => {
                if let Some(o) = self.open.as_mut() {
                    match r {
                        Ok(()) => {
                            o.full.highlights.retain(|h| h.id != id);
                            if o.editing == Some(id) {
                                o.editing = None;
                            }
                        }
                        Err(e) => o.flash = Some(e),
                    }
                }
                Task::none()
            }
            Msg::Cancel => {
                if let Some(o) = self.open.as_mut() {
                    o.sel = None;
                    o.editing = None;
                    o.flash = None;
                }
                Task::none()
            }
            Msg::Copy => {
                let Some((_, _, text)) = self.selection_text() else {
                    return Task::none();
                };
                if let Some(o) = self.open.as_mut() {
                    o.sel = None;
                    o.flash = Some("copied".into());
                }
                iced::clipboard::write(text)
            }
            Msg::Export => {
                let Some(o) = self.open.as_ref() else {
                    return Task::none();
                };
                let id = o.full.book.id;
                let c = c.clone();
                Task::perform(
                    async move {
                        let b = c.bytes(&format!("/api/books/{id}/highlights.md")).await?;
                        Ok(String::from_utf8_lossy(&b).to_string())
                    },
                    Msg::Exported,
                )
            }
            Msg::Exported(Ok(md)) => {
                if let Some(o) = self.open.as_mut() {
                    o.flash = Some("highlights and notes copied as Markdown".into());
                }
                iced::clipboard::write(md)
            }
            Msg::Bookmark => {
                let Some(o) = self.open.as_ref() else {
                    return Task::none();
                };
                let (id, page, block) = (
                    o.full.book.id,
                    o.page,
                    if o.full.book.format == "epub" {
                        o.block
                    } else {
                        0
                    },
                );
                let label = match &o.chapter {
                    Some(ch) => ch
                        .blocks
                        .get(block as usize)
                        .map(|b| b.spans.iter().map(|s| s.text.as_str()).collect::<String>())
                        .map(|t| t.split_whitespace().take(5).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default(),
                    None => String::new(),
                };
                let c = c.clone();
                Task::perform(
                    async move {
                        c.post(
                            &format!("/api/books/{id}/bookmarks"),
                            Some(
                                serde_json::json!({ "page": page, "block": block, "label": label }),
                            ),
                        )
                        .await
                    },
                    Msg::Bookmarked,
                )
            }
            Msg::Bookmarked(Ok(t)) => {
                if let Some(o) = self.open.as_mut() {
                    o.flash = Some(
                        if t.id.is_some() {
                            "bookmarked"
                        } else {
                            "bookmark taken off"
                        }
                        .into(),
                    );
                }
                self.refresh(c)
            }
            Msg::DeleteBookmark(id) => {
                let c2 = c.clone();
                Task::perform(
                    async move {
                        c2.send_ok(
                            reqwest::Method::DELETE,
                            &format!("/api/bookmarks/{id}"),
                            None,
                        )
                        .await
                    },
                    Msg::Saved,
                )
                .chain(self.refresh(c))
            }
            Msg::Side(s) => {
                if let Some(o) = self.open.as_mut() {
                    o.side = if o.side == s { Side::None } else { s };
                }
                Task::none()
            }
            Msg::Size(d) => {
                self.prefs.size = (self.prefs.size + d).clamp(14.0, 34.0);
                self.prefs.save();
                Task::none()
            }
            Msg::Line(v) => {
                self.prefs.line = v;
                self.prefs.save();
                Task::none()
            }
            Msg::Measure(v) => {
                self.prefs.measure = v;
                self.prefs.save();
                Task::none()
            }
            Msg::Dark => {
                self.prefs.dark = !self.prefs.dark;
                self.prefs.save();
                self.fetch_around(c)
            }
            Msg::Fit(f) => {
                self.prefs.pdf_fit = f;
                self.prefs.save();
                if let Some(o) = self.open.as_mut() {
                    o.pics.retain(|k, _| k.2);
                }
                self.fetch_around(c)
            }
            Msg::Spread => {
                self.prefs.spread = !self.prefs.spread;
                self.prefs.save();
                self.fetch_around(c)
            }
            Msg::Direction => {
                self.prefs.right_to_left = !self.prefs.right_to_left;
                self.prefs.save();
                Task::none()
            }
            Msg::RotateScreen => {
                self.prefs.rotate_screen = !self.prefs.rotate_screen;
                self.prefs.save();
                Task::none()
            }
            // Rotate turns and turns back; the other way flips while turned.
            Msg::Rotate => {
                let turned = self.turn != 0 || self.page_turn != 0;
                self.turn_to(if turned { 0 } else { self.dir })
            }
            Msg::RotateOther => {
                self.dir = 3 - self.dir;
                self.turn_to(self.dir)
            }
            Msg::Step(d) => {
                let Some(o) = self.open.as_ref() else {
                    return Task::none();
                };
                let epub = o.full.book.format == "epub";
                let wide = !o.full.book.is_comic() && self.prefs.pdf_fit == Fit::Width;
                if !epub && !wide {
                    return self.turn_page(c, d as i64);
                }
                // At the very end of a chapter, down goes on to the next one.
                if epub && d > 0.0 && o.rel >= 0.999 {
                    return self.turn_page(c, 1);
                }
                let step = if epub {
                    self.prefs.size * self.prefs.line * 3.0
                } else {
                    120.0
                };
                scrollable::scroll_by(
                    scrollable::Id::new(if epub { "chapter" } else { "page" }),
                    scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: d * step,
                    },
                )
            }
            Msg::Chrome => {
                if let Some(o) = self.open.as_mut() {
                    o.chrome = !o.chrome;
                }
                Task::none()
            }
        }
    }

    /// The window changed size: pages may need drawing at another width.
    pub fn resized(&mut self, c: &Client, w: f32, h: f32) -> Task<Msg> {
        let before = self.page_px(false);
        self.window = Size::new(w, h);
        if self.page_px(false) != before {
            if let Some(o) = self.open.as_mut() {
                o.pics.retain(|k, _| k.2);
                o.loading.clear();
            }
        }
        self.fetch_around(c)
    }

    /// A key while a book is open.
    pub fn key(key: iced::keyboard::Key) -> Option<Msg> {
        use iced::keyboard::{key::Named, Key};
        Some(match key.as_ref() {
            Key::Named(Named::ArrowRight | Named::PageDown | Named::Space) => Msg::Next,
            Key::Named(Named::ArrowLeft | Named::PageUp) => Msg::Prev,
            Key::Named(Named::ArrowDown) | Key::Character("j") => Msg::Step(1.0),
            Key::Named(Named::ArrowUp) | Key::Character("k") => Msg::Step(-1.0),
            Key::Character("b") => Msg::Bookmark,
            Key::Character("d") => Msg::Dark,
            Key::Character("r") => Msg::Rotate,
            Key::Character("m") => Msg::Side(Side::Marks),
            Key::Character("t") => Msg::Side(Side::Contents),
            Key::Character("+") | Key::Character("=") => Msg::Size(1.0),
            Key::Character("-") => Msg::Size(-1.0),
            Key::Character("1") => Msg::Mark("ochre"),
            Key::Character("2") => Msg::Mark("pink"),
            Key::Character("3") => Msg::Mark("green"),
            Key::Character("n") => Msg::Mark("note"),
            Key::Character("y") => Msg::Copy,
            Key::Character("f") => Msg::Chrome,
            _ => return None,
        })
    }
}

// ---- drawing --------------------------------------------------------------------------

const BAR_H: f32 = 56.0;
const FOOT_H: f32 = 44.0;
const SIDE_W: f32 = 300.0;
const LINE: Color = Color::from_rgba(1.0, 0.98, 0.74, 0.3);

fn chip<'a>(label: impl ToString, on: bool, m: Msg) -> Element<'a, Msg> {
    button(text(label.to_string()).size(12).font(MONO))
        .style(move |_t: &Theme, s: button::Status| {
            let hover = matches!(s, button::Status::Hovered | button::Status::Pressed);
            let fill = on || hover;
            button::Style {
                background: Some(if fill { CREAM } else { Color::TRANSPARENT }.into()),
                text_color: if fill { style::CARD } else { CREAM },
                border: iced::Border {
                    color: CREAM,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                shadow: Default::default(),
            }
        })
        .padding([7, 12])
        .on_press(m)
        .into()
}

fn link<'a>(label: impl ToString, m: Msg, color: Color) -> Element<'a, Msg> {
    button(text(label.to_string()).size(12).font(MONO).color(color))
        .style(style::plain_in(color))
        .padding(0)
        .on_press(m)
        .into()
}

fn swatch_button<'a>(c: Color, m: Msg) -> Element<'a, Msg> {
    button(Space::new(Length::Fixed(22.0), Length::Fixed(22.0)))
        .style(move |_t: &Theme, s: button::Status| button::Style {
            background: Some(c.into()),
            border: iced::Border {
                color: if matches!(s, button::Status::Hovered) {
                    CREAM
                } else {
                    c
                },
                width: 2.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .padding(0)
        .on_press(m)
        .into()
}

fn mono<'a>(s: impl ToString, size: f32, c: Color) -> iced::widget::Text<'a> {
    text(s.to_string()).size(size).font(MONO).color(c)
}

/// A cover, or the title set on a colour block when there is none.
pub fn cover<'a, M: 'a>(r: &Reader, b: &Book, w: f32) -> Element<'a, M> {
    let h = (w * 1.5).round();
    if let Some(p) = b.cover_id.and_then(|id| r.covers.get(&id)) {
        return container(
            image(p.handle.clone())
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .content_fit(iced::ContentFit::Contain),
        )
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .center_x(Length::Fixed(w))
        .style(style::filled(style::CARD))
        .into();
    }
    let (bg, fg, acc) = style::placeholder_colors(b.id);
    container(
        column![
            text(b.label())
                .size(style::title_card_size(&b.label(), w * 0.84, w * 0.13).clamp(11.0, 40.0))
                .font(BLACK)
                .line_height(1.0)
                .color(fg),
            vertical_space(),
            container(Space::new(
                Length::Fixed((w - w * 0.16) * b.progress.clamp(0.0, 1.0) as f32),
                Length::Fixed(3.0)
            ))
            .style(style::filled(acc)),
        ]
        .height(Length::Fill),
    )
    .padding(w * 0.08)
    .style(style::filled_text(bg, fg))
    .width(Length::Fixed(w))
    .height(Length::Fixed(h))
    .clip(true)
    .into()
}

/// The canvas over a page: highlights, the selection, and the mouse.
struct PageMarks<'a> {
    page: i64,
    words: &'a [Word],
    marks: Vec<(Vec<[f32; 4]>, Color)>,
    sel: Option<(usize, usize)>,
    selectable: bool,
    /// The window's width, to tell which half of it a tap was in.
    window_w: f32,
}

#[derive(Default)]
struct Drag {
    from: Option<Point>,
    to: Option<Point>,
    /// A finger on the page: where it went down (window coordinates), and
    /// whether it has moved far enough to be a scroll rather than a tap.
    finger: Option<(Point, bool)>,
}

fn frac(p: Point, b: Rectangle) -> Point {
    Point::new(
        ((p.x - b.x) / b.width).clamp(0.0, 1.0),
        ((p.y - b.y) / b.height).clamp(0.0, 1.0),
    )
}

impl Program<Msg> for PageMarks<'_> {
    type State = Drag;

    fn update(
        &self,
        st: &mut Drag,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Msg>) {
        use canvas::event::Status;
        use iced::touch;
        // Fingers first: they come without a cursor. A tap turns the page or
        // opens a highlight; a drag is left alone, so the page scrolls under it.
        match event {
            canvas::Event::Touch(touch::Event::FingerPressed { position, .. })
                if bounds.contains(position) =>
            {
                st.finger = Some((position, false));
                return (Status::Ignored, None);
            }
            canvas::Event::Touch(touch::Event::FingerMoved { position, .. }) => {
                if let Some((at, moved)) = st.finger.as_mut() {
                    if (position.x - at.x).abs() + (position.y - at.y).abs() > 12.0 {
                        *moved = true;
                    }
                }
                return (Status::Ignored, None);
            }
            canvas::Event::Touch(touch::Event::FingerLifted { position, .. }) => {
                return match st.finger.take() {
                    Some((_, false)) if bounds.contains(position) => (
                        Status::Captured,
                        Some(Msg::PageClick(
                            self.page,
                            frac(position, bounds),
                            position.x / self.window_w.max(1.0),
                        )),
                    ),
                    _ => (Status::Ignored, None),
                };
            }
            canvas::Event::Touch(touch::Event::FingerLost { .. }) => {
                st.finger = None;
                return (Status::Ignored, None);
            }
            _ => {}
        }
        let Some(pos) = cursor.position() else {
            return (Status::Ignored, None);
        };
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if bounds.contains(pos) =>
            {
                st.from = Some(frac(pos, bounds));
                st.to = st.from;
                (Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if st.from.is_some() => {
                st.to = Some(frac(pos, bounds));
                (Status::Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (Some(a), Some(b)) = (st.from.take(), st.to.take()) else {
                    return (Status::Ignored, None);
                };
                let moved = (a.x - b.x).abs() + (a.y - b.y).abs() > 0.01;
                let msg = if moved && self.selectable {
                    Msg::PageSelect(self.page, a, b)
                } else {
                    Msg::PageClick(self.page, b, pos.x / self.window_w.max(1.0))
                };
                (Status::Captured, Some(msg))
            }
            _ => (Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        st: &Drag,
        renderer: &Renderer,
        _t: &Theme,
        bounds: Rectangle,
        _c: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let fill = |frame: &mut Frame, r: &[f32; 4], c: Color| {
            frame.fill_rectangle(
                Point::new(r[0] * w - 2.0, r[1] * h - 2.0),
                Size::new(r[2] * w + 4.0, r[3] * h + 4.0),
                c,
            );
        };
        for (rs, c) in &self.marks {
            for r in rs {
                fill(&mut frame, r, alpha(*c, 0.38));
            }
        }
        // While dragging, the words under the drag; after, the selection.
        let range = match (st.from, st.to) {
            (Some(a), Some(b)) if self.selectable => {
                match (word_at(self.words, a), word_at(self.words, b)) {
                    (Some(x), Some(y)) => Some((x.min(y), x.max(y))),
                    _ => None,
                }
            }
            _ => self.sel,
        };
        if let Some((a, b)) = range {
            for r in rects(self.words, a, b) {
                fill(&mut frame, &r, alpha(CREAM, 0.35));
            }
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _s: &Drag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            if self.selectable {
                mouse::Interaction::Text
            } else {
                mouse::Interaction::Pointer
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl Reader {
    // ---- the Read page ---------------------------------------------------------

    pub fn home_view(&self) -> Element<'_, Msg> {
        let now = self
            .home
            .reading
            .first()
            .and_then(|b| self.book(b.id).or(Some(b)));
        let ink = BOOKS_INK;
        let mut left = column![row![
            text("Reading now").size(34).font(BLACK).color(ink),
            Space::with_width(14),
            mono(
                now.map(|_| "where you left off")
                    .unwrap_or("open a book from the catalogue"),
                12.0,
                ink
            ),
        ]
        .align_y(Alignment::End)]
        .spacing(16);
        left = left.push(horizontal_rule(1).style(style::hairline(ink)));
        if let Some(b) = now {
            let how = if b.format == "epub" {
                format!("EPUB · chapter {} of {}", b.page + 1, b.pages.max(1))
            } else {
                format!(
                    "{} · page {} of {}",
                    b.format.to_uppercase(),
                    b.page + 1,
                    b.pages.max(1)
                )
            };
            let bar = row![
                container(Space::new(
                    Length::FillPortion((b.progress * 1000.0).round().max(1.0) as u16),
                    Length::Fixed(6.0)
                ))
                .style(style::filled(ink)),
                container(Space::new(
                    Length::FillPortion(((1.0 - b.progress) * 1000.0).round().max(1.0) as u16),
                    Length::Fixed(6.0)
                ))
                .style(style::filled(alpha(ink, 0.25))),
            ];
            let continue_btn = button(text("Continue").size(15).font(BLACK).color(BOOKS))
                .style(move |_t: &Theme, _s| button::Style {
                    background: Some(ink.into()),
                    text_color: BOOKS,
                    ..Default::default()
                })
                .padding([10, 18])
                .on_press(Msg::Open(b.id));
            let info = column![
                text(b.label())
                    .size(30)
                    .font(BLACK)
                    .line_height(1.05)
                    .color(ink),
                text(
                    [b.author.clone(), b.year.map(|y| y.to_string())]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ")
                )
                .size(16)
                .color(ink),
                mono(how, 12.0, ink),
                Space::with_height(4),
                bar,
                mono(format!("{}%", (b.progress * 100.0).round()), 12.0, ink),
                Space::with_height(6),
                row![
                    continue_btn,
                    mono(
                        format!(
                            "{} highlight{} · {} bookmark{}",
                            b.highlight_count,
                            if b.highlight_count == 1 { "" } else { "s" },
                            b.bookmark_count,
                            if b.bookmark_count == 1 { "" } else { "s" }
                        ),
                        12.0,
                        ink
                    )
                ]
                .spacing(14)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .width(Length::Fill);
            left = left.push(row![cover(self, b, 200.0), info].spacing(28));
            let also: Vec<&Book> = self.home.reading.iter().skip(1).take(4).collect();
            if !also.is_empty() {
                left = left
                    .push(Space::with_height(6))
                    .push(text("Also open").size(20).font(BLACK).color(ink));
                for b in also {
                    let where_ = if b.format == "epub" {
                        format!("EPUB · chapter {}", b.page + 1)
                    } else {
                        format!(
                            "{} · p. {} of {}",
                            b.format.to_uppercase(),
                            b.page + 1,
                            b.pages
                        )
                    };
                    left = left.push(
                        button(column![
                            row![
                                text(b.label())
                                    .size(16)
                                    .font(BOLD)
                                    .color(ink)
                                    .width(Length::Fill),
                                mono(where_, 12.0, ink),
                                Space::with_width(12),
                                mono(format!("{}%", (b.progress * 100.0).round()), 12.0, ink),
                            ]
                            .align_y(Alignment::End),
                            Space::with_height(6),
                            horizontal_rule(1).style(style::hairline(alpha(ink, 0.35))),
                        ])
                        .style(style::plain_in(ink))
                        .padding(0)
                        .on_press(Msg::Open(b.id)),
                    );
                }
            }
        } else {
            left = left.push(
                text(
                    "Nothing open yet. katalogd reads Books/ and Comics/ next to Music/; \
                     a book you open here shows up on this page.",
                )
                .size(15)
                .color(ink),
            );
        }
        let left = container(scrollable(left).style(style::scroll(ink)))
            .padding([32, 40])
            .style(style::filled_text(BOOKS, ink))
            .width(Length::FillPortion(1))
            .height(Length::Fill);

        let mut marks = column![
            row![
                text("Highlights").size(34).font(BLACK),
                Space::with_width(14),
                mono("latest first", 12.0, alpha(CREAM, 0.8)),
            ]
            .align_y(Alignment::End),
            horizontal_rule(1).style(style::hairline(CREAM)),
        ]
        .spacing(14);
        if self.home.highlights.is_empty() {
            marks = marks.push(
                text("Select words in a book and give them a colour; they gather here with their notes.")
                    .size(15)
                    .color(alpha(CREAM, 0.85)),
            );
        }
        for h in self.home.highlights.iter().take(5) {
            let mut body =
                column![text(h.text.clone()).size(18).font(SERIF).line_height(1.45)].spacing(6);
            if !h.note.is_empty() {
                body = body.push(mono(h.note.clone(), 13.0, CREAM));
            }
            let place = match self.book(h.book_id) {
                Some(b) if b.format == "epub" => format!("chapter {}", h.page + 1),
                _ => format!("p. {}", h.page + 1),
            };
            body = body.push(mono(
                format!("{} · {place}", h.book.clone().unwrap_or_default()),
                12.0,
                alpha(CREAM, 0.75),
            ));
            let block = Space::new(Length::Fixed(4.0), Length::Fill);
            let item = row![
                container(block).style(style::filled(mark_color(&h.color))),
                body
            ]
            .spacing(14)
            .height(Length::Shrink);
            let (bid, page) = (h.book_id, h.page);
            marks = marks.push(
                mouse_area(column![
                    item,
                    Space::with_height(10),
                    horizontal_rule(1).style(style::hairline(LINE))
                ])
                .on_press(Msg::Open(bid))
                .interaction(mouse::Interaction::Pointer),
            );
            let _ = page;
        }

        // Comics on the pile: issues started first, then the unread.
        let mut pile: Vec<&Book> = self
            .books
            .iter()
            .filter(|b| b.is_comic() && b.progress < 0.995)
            .collect();
        pile.sort_by_key(|b| (b.progress == 0.0, std::cmp::Reverse(b.added_at.clone())));
        let cw = ((self.window.width / 2.0 - 80.0 - 4.0 * 16.0) / 5.0).clamp(60.0, 160.0);
        let mut strip = row![].spacing(16);
        for b in pile.iter().take(5) {
            strip = strip.push(
                mouse_area(
                    column![
                        cover(self, b, cw),
                        text(b.label()).size(13).font(BOLD).width(Length::Fixed(cw)),
                        mono(b.byline(), 11.0, CREAM).width(Length::Fixed(cw)),
                    ]
                    .spacing(4),
                )
                .on_press(Msg::Open(b.id))
                .interaction(mouse::Interaction::Pointer),
            );
        }
        let comics = container(
            column![
                row![
                    text("Comics on the pile").size(20).font(BLACK),
                    Space::with_width(14),
                    mono(
                        if pile.is_empty() {
                            "none yet"
                        } else {
                            "started first, then the new ones"
                        },
                        12.0,
                        CREAM
                    ),
                ]
                .align_y(Alignment::End),
                strip,
            ]
            .spacing(12),
        )
        .padding([20, 40])
        .width(Length::Fill)
        .style(style::filled_text(COMICS, CREAM));

        let right = column![
            container(scrollable(marks).style(style::scroll(CREAM)))
                .padding([32, 40])
                .height(Length::Fill)
                .width(Length::Fill)
                .style(style::filled_text(style::CARD, CREAM)),
            comics,
        ]
        .width(Length::FillPortion(1))
        .height(Length::Fill);
        row![left, right].height(Length::Fill).into()
    }

    // ---- the reader -------------------------------------------------------------

    pub fn reader_view(&self) -> Element<'_, Msg> {
        let Some(o) = self.open.as_ref() else {
            return Space::new(Length::Fill, Length::Fill).into();
        };
        let b = &o.full.book;
        let epub = b.format == "epub";
        let comic = b.is_comic();
        let (page_bg, ink) = if self.prefs.dark || comic {
            (PAGE_DARK, TYPE_DARK)
        } else {
            (PAGE_PAPER, TYPE_PAPER)
        };

        // The bar.
        let where_ = if epub {
            o.chapter
                .as_ref()
                .map(|c| c.title.clone())
                .or_else(|| {
                    o.full
                        .chapters
                        .get(o.page as usize)
                        .map(|c| c.title.clone())
                })
                .unwrap_or_default()
        } else {
            let shown = self.shown_pages(o);
            let (lo, hi) = (
                shown.iter().min().copied().unwrap_or(0),
                shown.iter().max().copied().unwrap_or(0),
            );
            if lo == hi {
                format!("page {} of {}", lo + 1, b.pages)
            } else {
                format!("pages {}–{} of {}", lo + 1, hi + 1, b.pages)
            }
        };
        let marked = o
            .full
            .bookmarks
            .iter()
            .any(|k| k.page == o.page && (!epub || k.block == o.block));
        let mut bar = row![
            link("← Read", Msg::Close, CREAM),
            text(b.label()).size(17).font(BLACK),
            mono(
                [b.author.clone().map(|a| surname(&a)), Some(where_)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                13.0,
                alpha(CREAM, 0.8)
            ),
            horizontal_space(),
        ]
        .spacing(18)
        .align_y(Alignment::Center);
        if !epub {
            bar = bar.push(mono("go to", 12.0, alpha(CREAM, 0.6))).push(
                text_input(&format!("{}", o.page + 1), &o.goto)
                    .on_input(Msg::GotoInput)
                    .on_submit(Msg::GotoSubmit)
                    .size(12)
                    .font(MONO)
                    .width(Length::Fixed(54.0))
                    .padding([6, 8])
                    .style(style::search),
            );
        }
        if epub {
            bar = bar.push(
                row![
                    chip("A−", false, Msg::Size(-1.0)),
                    chip("A+", false, Msg::Size(1.0))
                ]
                .spacing(4),
            );
        } else if !comic {
            bar = bar.push(
                row![
                    chip(
                        "Fit width",
                        self.prefs.pdf_fit == Fit::Width,
                        Msg::Fit(Fit::Width)
                    ),
                    chip(
                        "Fit page",
                        self.prefs.pdf_fit == Fit::Page,
                        Msg::Fit(Fit::Page)
                    ),
                    chip(
                        "Two pages",
                        self.prefs.pdf_fit == Fit::Two,
                        Msg::Fit(Fit::Two)
                    ),
                ]
                .spacing(0),
            );
        } else {
            bar = bar.push(
                row![
                    chip("Spread", self.prefs.spread, Msg::Spread),
                    chip(
                        if self.prefs.right_to_left {
                            "Right to left"
                        } else {
                            "Left to right"
                        },
                        false,
                        Msg::Direction
                    ),
                ]
                .spacing(4),
            );
        }
        if !comic {
            bar = bar.push(row![
                chip("Dark", self.prefs.dark, Msg::Dark),
                chip("Paper", !self.prefs.dark, Msg::Dark)
            ]);
        }
        let turned = self.turn != 0 || self.page_turn != 0;
        if turned {
            bar = bar.push(
                row![
                    chip("Turn back", true, Msg::Rotate),
                    chip("Other way", false, Msg::RotateOther),
                ]
                .spacing(4),
            );
        } else {
            bar = bar.push(chip("Rotate", false, Msg::Rotate));
        }
        bar = bar
            .push(chip(
                if marked { "Bookmarked" } else { "Bookmark" },
                marked,
                Msg::Bookmark,
            ))
            .push(chip(
                format!("Marks {}", o.full.highlights.len() + o.full.bookmarks.len()),
                o.side != Side::None,
                Msg::Side(Side::Marks),
            ));
        let bar = container(bar)
            .padding([0, 24])
            .height(Length::Fixed(BAR_H))
            .center_y(Length::Fixed(BAR_H));

        // The page.
        let side_w = if o.side != Side::None { SIDE_W } else { 0.0 };
        let area = Size::new(
            (self.window.width - side_w).max(200.0),
            (self.window.height - BAR_H - FOOT_H - 2.0).max(200.0),
        );
        let body: Element<Msg> = if epub {
            self.chapter_view(o, page_bg, ink, area)
        } else {
            self.pages_view(o, area)
        };
        let body = container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::filled(if epub { page_bg } else { style::CARD }));

        let mut middle = row![];
        if o.side != Side::None {
            middle = middle.push(self.side_view(o)).push(
                container(Space::new(Length::Fixed(1.0), Length::Fill)).style(style::filled(LINE)),
            );
        }
        middle = middle.push(body);

        let show_chrome = !comic || o.chrome;
        let mut page = column![];
        if show_chrome {
            page = page
                .push(bar)
                .push(horizontal_rule(1).style(style::hairline(LINE)));
        }
        page = page.push(container(middle).height(Length::Fill));
        if show_chrome {
            page = page
                .push(horizontal_rule(1).style(style::hairline(LINE)))
                .push(self.foot_view(o));
        }
        container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::filled_text(style::CARD, CREAM))
            .into()
    }

    fn foot_view<'a>(&'a self, o: &'a Open) -> Element<'a, Msg> {
        let b = &o.full.book;
        // The selection waiting for a colour comes first.
        let content: Element<Msg> = if o.sel.is_some() {
            let waiting = matches!(o.sel, Some(Sel::Text { done: false, .. }));
            row![
                mono(
                    if waiting {
                        "click the last word, or pick a colour for one word"
                    } else {
                        "highlight"
                    },
                    12.0,
                    alpha(CREAM, 0.8)
                ),
                swatch_button(style::OCHRE, Msg::Mark("ochre")),
                swatch_button(style::PINK, Msg::Mark("pink")),
                swatch_button(style::GREEN, Msg::Mark("green")),
                chip("Note n", false, Msg::Mark("note")),
                chip("Copy y", false, Msg::Copy),
                link("cancel", Msg::Cancel, alpha(CREAM, 0.8)),
                horizontal_space(),
                mono("1 2 3 colour", 12.0, alpha(CREAM, 0.6)),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        } else if b.is_comic() {
            // A strip of the pages around this one.
            let mut strip = row![].spacing(8).align_y(Alignment::End);
            let here = self.shown_pages(o);
            for p in (o.page - 6).max(0)..=(o.page + 8).min(b.pages - 1) {
                let lit = here.contains(&p);
                let thumb: Element<Msg> = match o.pics.get(&(p, false, true)) {
                    Some(pic) => image(pic.handle.clone())
                        .width(Length::Fixed(24.0))
                        .height(Length::Fixed(34.0))
                        .content_fit(iced::ContentFit::Cover)
                        .opacity(if lit { 1.0_f32 } else { 0.45 })
                        .into(),
                    None => container(Space::new(Length::Fixed(24.0), Length::Fixed(34.0)))
                        .style(style::filled(alpha(CREAM, if lit { 0.8 } else { 0.25 })))
                        .into(),
                };
                strip = strip.push(
                    mouse_area(thumb)
                        .on_press(Msg::Go(p))
                        .interaction(mouse::Interaction::Pointer),
                );
            }
            row![
                strip,
                horizontal_space(),
                mono(
                    o.flash
                        .clone()
                        .unwrap_or_else(|| "space or → on · ← back · f hides the bar".into()),
                    12.0,
                    alpha(CREAM, 0.7)
                ),
            ]
            .align_y(Alignment::Center)
            .into()
        } else {
            let progress = self.progress();
            let ticks: Vec<f32> = o
                .full
                .bookmarks
                .iter()
                .map(|k| (k.page as f32 + 0.5) / b.pages.max(1) as f32)
                .collect();
            let line = progress_line(progress as f32, ticks);
            let label = if b.format == "epub" {
                format!("chapter {} of {}", o.page + 1, b.pages)
            } else {
                format!("page {} of {}", o.page + 1, b.pages)
            };
            let right = o.flash.clone().unwrap_or_else(|| {
                if b.format == "epub" {
                    format!(
                        "{}% · Literata {} · line {}",
                        (progress * 100.0).round(),
                        self.prefs.size,
                        self.prefs.line
                    )
                } else {
                    format!(
                        "{}% · drag over the text to highlight",
                        (progress * 100.0).round()
                    )
                }
            });
            row![
                link("‹", Msg::Prev, CREAM),
                mono(label, 12.0, alpha(CREAM, 0.8)),
                line,
                mono(right, 12.0, alpha(CREAM, 0.8)),
                link("›", Msg::Next, CREAM),
            ]
            .spacing(16)
            .align_y(Alignment::Center)
            .into()
        };
        container(content)
            .padding([0, 24])
            .height(Length::Fixed(FOOT_H))
            .center_y(Length::Fixed(FOOT_H))
            .width(Length::Fill)
            .into()
    }

    fn chapter_view<'a>(
        &'a self,
        o: &'a Open,
        page_bg: Color,
        ink: Color,
        area: Size,
    ) -> Element<'a, Msg> {
        let Some(ch) = o.chapter.as_ref() else {
            return container(mono("opening…", 13.0, alpha(ink, 0.7)))
                .center(Length::Fill)
                .into();
        };
        let size = self.prefs.size;
        let width = (size * self.prefs.measure * 0.5)
            .min(area.width - 48.0)
            .max(240.0);
        let sel = match &o.sel {
            Some(Sel::Text { a, b, .. }) => Some(ord(*a, *b)),
            _ => None,
        };
        let marks: Vec<((usize, usize), (usize, usize), Color, i64)> = o
            .full
            .highlights
            .iter()
            .filter(|h| h.page == o.page)
            .filter_map(|h| match h.anchor {
                Anchor::Text { b0, w0, b1, w1 } => Some((
                    (b0 as usize, w0 as usize),
                    (b1 as usize, w1 as usize),
                    mark_color(&h.color),
                    h.id,
                )),
                _ => None,
            })
            .collect();
        // Notes sit in the margin when there is room for them.
        let note_w = ((area.width - width) / 2.0 - 40.0).min(240.0);
        let in_margin = note_w > 160.0;
        let extra = if in_margin { 32.0 + note_w } else { 0.0 };
        let fixed = |e: Element<'a, Msg>| -> Element<'a, Msg> {
            container(e).width(Length::Fixed(width)).into()
        };
        let mut col = column![]
            .spacing(size * 0.9)
            .width(Length::Fixed(width + extra));
        if o.page > 0 {
            col = col.push(fixed(
                link("← previous chapter", Msg::Prev, alpha(ink, 0.7)).into(),
            ));
        }
        let noted: HashSet<i64> = o
            .full
            .highlights
            .iter()
            .filter(|h| !h.note.is_empty())
            .map(|h| h.id)
            .collect();
        for (bi, block) in ch.blocks.iter().enumerate() {
            if block.kind == "img" {
                if let Some(p) = block.src.as_ref().and_then(|s| o.res.get(s)) {
                    let w = (p.w as f32 / self.scale).min(width);
                    col = col.push(fixed(
                        container(image(p.handle.clone()).width(Length::Fixed(w)))
                            .center_x(Length::Fill)
                            .into(),
                    ));
                }
                continue;
            }
            if block.kind == "hr" {
                col = col.push(fixed(
                    container(mono("·  ·  ·", size * 0.8, alpha(ink, 0.6)))
                        .center_x(Length::Fill)
                        .into(),
                ));
                continue;
            }
            let (fsize, heading) = match block.kind.as_str() {
                "h1" => (size * 1.45, true),
                "h2" => (size * 1.25, true),
                "h3" => (size * 1.1, true),
                _ => (size, false),
            };
            let toks = o.toks.get(bi).map(Vec::as_slice).unwrap_or(&[]);
            let spans: Vec<iced::widget::text::Span<'a, Msg, iced::Font>> = toks
                .iter()
                .map(|t| {
                    let p = (bi, t.word);
                    let mark = marks.iter().find(|(a, b, _, _)| in_range(p, *a, *b));
                    let bg = if sel.is_some_and(|(a, b)| in_range(p, a, b)) {
                        Some(alpha(
                            if self.prefs.dark { CREAM } else { style::NAVY },
                            0.30,
                        ))
                    } else {
                        mark.map(|(_, _, c, _)| {
                            alpha(*c, if self.prefs.dark { 0.42 } else { 0.55 })
                        })
                    };
                    let font = style::literata(
                        if t.bold || heading {
                            iced::font::Weight::Semibold
                        } else {
                            iced::font::Weight::Normal
                        },
                        t.italic,
                    );
                    // A highlight with a note is underlined.
                    let noted_here = mark.is_some_and(|m| noted.contains(&m.3));
                    span(t.text.clone())
                        .font(font)
                        .color(ink)
                        .underline(noted_here)
                        .background_maybe(bg)
                        .link(Msg::Word(bi, t.word))
                })
                .collect();
            let para = rich_text(spans)
                .size(fsize)
                .line_height(self.prefs.line)
                .width(Length::Fill);
            let el: Element<Msg> = match block.kind.as_str() {
                "quote" => container(para)
                    .padding(iced::Padding {
                        left: size * 1.5,
                        ..Default::default()
                    })
                    .into(),
                "li" => row![text("•").size(fsize).font(SERIF).color(ink), para]
                    .spacing(size * 0.6)
                    .into(),
                _ => para.into(),
            };
            // A note beside its paragraph's first highlight, in the margin when there is room.
            let note = o
                .full
                .highlights
                .iter()
                .filter(|h| h.page == o.page && !h.note.is_empty())
                .find(|h| matches!(h.anchor, Anchor::Text { b0, .. } if b0 as usize == bi));
            match note {
                Some(h) if in_margin => {
                    let n = column![
                        container(Space::new(Length::Fill, Length::Fixed(2.0)))
                            .style(style::filled(mark_color(&h.color))),
                        mono("Note", 12.0, mark_color(&h.color)),
                        mono(h.note.clone(), 12.0, alpha(ink, 0.9)),
                    ]
                    .spacing(6)
                    .width(Length::Fixed(note_w));
                    let id = h.id;
                    col = col.push(
                        row![
                            container(el).width(Length::Fixed(width)),
                            mouse_area(n)
                                .on_press(Msg::OpenNote(id))
                                .interaction(mouse::Interaction::Pointer),
                        ]
                        .spacing(32),
                    );
                }
                Some(h) => {
                    let id = h.id;
                    col = col.push(fixed(el)).push(
                        mouse_area(mono(
                            format!("note: {}", h.note),
                            12.0,
                            mark_color(&h.color),
                        ))
                        .on_press(Msg::OpenNote(id))
                        .interaction(mouse::Interaction::Pointer),
                    );
                }
                None => col = col.push(fixed(el)),
            }
        }
        if (o.page + 1) < o.full.book.pages {
            let next = o
                .full
                .chapters
                .get(o.page as usize + 1)
                .map(|c| c.title.clone())
                .unwrap_or_default();
            col = col.push(Space::with_height(size)).push(
                button(
                    text(format!("Next: {next} →"))
                        .size(size * 0.85)
                        .font(SERIF_BOLD)
                        .color(ink),
                )
                .style(style::plain_in(ink))
                .padding(0)
                .on_press(Msg::Next),
            );
        } else {
            col = col
                .push(Space::with_height(size))
                .push(mono("the end", 13.0, alpha(ink, 0.7)));
        }
        let col = col.push(Space::with_height(area.height * 0.4));
        // Wider rows for notes in the margin: centre the text column itself.
        let margin = ((area.width - width) / 2.0).max(24.0);
        let page = scrollable(container(col).padding(iced::Padding {
            top: 56.0,
            left: margin,
            right: 24.0,
            bottom: 0.0,
        }))
        .id(scrollable::Id::new("chapter"))
        .on_scroll(Msg::Scrolled)
        .style(style::scroll(alpha(ink, 0.6)))
        .width(Length::Fill)
        .height(Length::Fill);
        container(page).style(style::filled(page_bg)).into()
    }

    fn pages_view<'a>(&'a self, o: &'a Open, area: Size) -> Element<'a, Msg> {
        let shown = self.shown_pages(o);
        let comic = o.full.book.is_comic();
        let dark = self.dark_pages(o);
        let fit_width = !comic && self.prefs.pdf_fit == Fit::Width;
        let rotated = self.page_turn % 2 == 1;
        let n = shown.len() as f32;
        let gap = 4.0;
        let mut pages = row![].spacing(gap).align_y(Alignment::Center);
        for &p in &shown {
            // While a big page is on its way, its small one stands in, so
            // there is something to look at at once.
            let pic = o.pics.get(&(p, dark, false)).or_else(|| {
                if comic {
                    o.pics.get(&(p, false, true))
                } else {
                    None
                }
            });
            let Some(pic) = pic else {
                let (w, h) = if fit_width {
                    (area.width.min(1000.0), area.width.min(1000.0) * 1.41)
                } else {
                    (area.height / 1.41, area.height)
                };
                pages = pages.push(
                    container(mono(format!("page {}", p + 1), 12.0, alpha(CREAM, 0.6)))
                        .center_x(Length::Fixed(
                            w / n.max(1.0) * if fit_width { 1.0 } else { n },
                        ))
                        .center_y(Length::Fixed(h)),
                );
                continue;
            };
            let (pw, ph) = if rotated {
                (pic.h as f32, pic.w as f32)
            } else {
                (pic.w as f32, pic.h as f32)
            };
            let aspect = ph / pw.max(1.0);
            // Fit width scrolls down the page; the rest fit the whole page in.
            let (w, h) = if fit_width {
                let w = (area.width - 48.0).min(1100.0 * n) / n;
                (w, w * aspect)
            } else {
                let avail_w = (area.width - 16.0 - gap * (n - 1.0)) / n;
                let h = (area.height - 8.0).min(avail_w * aspect);
                (h / aspect, h)
            };
            let img = image(pic.handle.clone())
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .content_fit(iced::ContentFit::Fill)
                .rotation(iced::Rotation::Solid(iced::Radians(
                    self.page_turn as f32 * std::f32::consts::FRAC_PI_2,
                )));
            let words: &[Word] = o.words.get(&p).map(Vec::as_slice).unwrap_or(&[]);
            let marks: Vec<(Vec<[f32; 4]>, Color)> = o
                .full
                .highlights
                .iter()
                .filter(|h| h.page == p)
                .filter_map(|h| match &h.anchor {
                    Anchor::Page { rects, .. } => Some((rects.clone(), mark_color(&h.color))),
                    _ => None,
                })
                .collect();
            let sel = match &o.sel {
                Some(Sel::Page { page, first, last }) if *page == p => Some((*first, *last)),
                _ => None,
            };
            let overlay = Canvas::new(PageMarks {
                page: p,
                words,
                marks,
                sel,
                selectable: !comic && self.page_turn == 0,
                window_w: self.window.width,
            })
            .width(Length::Fixed(w))
            .height(Length::Fixed(h));
            let layered: Element<Msg> = if self.page_turn == 0 || comic {
                iced::widget::stack![img, overlay].into()
            } else {
                img.into()
            };
            pages = pages.push(layered);
        }
        let pages = container(pages)
            .center_x(Length::Fill)
            .padding([if fit_width { 24 } else { 4 }, 0]);
        if fit_width {
            scrollable(pages)
                .id(scrollable::Id::new("page"))
                .style(style::scroll(alpha(CREAM, 0.6)))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(pages).center(Length::Fill).into()
        }
    }

    fn side_view<'a>(&'a self, o: &'a Open) -> Element<'a, Msg> {
        let epub = o.full.book.format == "epub";
        let tab = |label: String, s: Side| -> Element<'a, Msg> {
            let on = o.side == s;
            button(column![
                text(label)
                    .size(13)
                    .font(if on { MONO_MEDIUM } else { MONO })
                    .color(if on { CREAM } else { alpha(CREAM, 0.7) }),
                Space::with_height(6),
                horizontal_rule(2).style(style::rule_px(
                    if on { CREAM } else { Color::TRANSPARENT },
                    2
                )),
            ])
            .style(style::plain)
            .padding(0)
            .on_press(Msg::Side(s))
            .into()
        };
        let mut tabs = row![].spacing(16);
        if epub {
            tabs = tabs.push(tab("Contents".into(), Side::Contents));
        }
        tabs = tabs.push(tab(
            format!("Marks {}", o.full.highlights.len() + o.full.bookmarks.len()),
            Side::Marks,
        ));
        let mut col = column![tabs, horizontal_rule(1).style(style::hairline(CREAM))].spacing(12);

        if o.side == Side::Contents && epub {
            for (i, ch) in o.full.chapters.iter().enumerate() {
                let here = i as i64 == o.page;
                col = col.push(
                    button(
                        text(ch.title.clone())
                            .size(14)
                            .font(if here { SERIF_BOLD } else { SERIF })
                            .color(if here { CREAM } else { alpha(CREAM, 0.8) }),
                    )
                    .style(style::plain)
                    .padding([2, 0])
                    .on_press(Msg::Go(i as i64)),
                );
            }
        } else {
            // The note being written, first.
            if let Some(h) = o
                .editing
                .and_then(|id| o.full.highlights.iter().find(|h| h.id == id))
            {
                let c = mark_color(&h.color);
                let id = h.id;
                let mut colours = row![].spacing(6);
                for name in COLORS {
                    colours = colours.push(swatch_button(mark_color(name), Msg::Recolor(id, name)));
                }
                col = col.push(
                    column![
                        container(Space::new(Length::Fill, Length::Fixed(2.0)))
                            .style(style::filled(c)),
                        text(h.text.clone()).size(15).font(SERIF).line_height(1.45),
                        mono("Note", 12.0, c),
                        text_editor(&o.note)
                            .on_action(Msg::NoteEdit)
                            .placeholder("what it makes you think")
                            .font(MONO)
                            .size(13)
                            .height(Length::Fixed(110.0))
                            .style(|_t: &Theme, _s| text_editor::Style {
                                background: Color::TRANSPARENT.into(),
                                border: iced::Border {
                                    color: alpha(CREAM, 0.5),
                                    width: 1.0,
                                    radius: 0.0.into()
                                },
                                icon: CREAM,
                                placeholder: alpha(CREAM, 0.5),
                                value: CREAM,
                                selection: alpha(CREAM, 0.3),
                            }),
                        row![
                            colours,
                            horizontal_space(),
                            chip("Save", true, Msg::SaveNote)
                        ]
                        .align_y(Alignment::Center),
                        row![
                            link(
                                "delete highlight",
                                Msg::DeleteHighlight(id),
                                alpha(CREAM, 0.7)
                            ),
                            horizontal_space(),
                            link("close", Msg::Cancel, alpha(CREAM, 0.7))
                        ],
                    ]
                    .spacing(8),
                );
                col = col.push(Space::with_height(8));
            }
            col = col.push(text("Bookmarks").size(15).font(BLACK));
            if o.full.bookmarks.is_empty() {
                col = col.push(mono("b puts one here", 12.0, alpha(CREAM, 0.7)));
            }
            for k in &o.full.bookmarks {
                let place = if epub {
                    o.full
                        .chapters
                        .get(k.page as usize)
                        .map(|c| c.title.clone())
                        .unwrap_or_else(|| format!("ch. {}", k.page + 1))
                } else {
                    format!("p. {}", k.page + 1)
                };
                let label = if k.label.is_empty() {
                    place.clone()
                } else {
                    k.label.clone()
                };
                col = col.push(
                    row![
                        button(row![
                            mono(label, 12.0, CREAM).width(Length::Fill),
                            mono(
                                if k.label.is_empty() {
                                    String::new()
                                } else {
                                    place
                                },
                                12.0,
                                alpha(CREAM, 0.7)
                            )
                        ])
                        .style(style::plain)
                        .padding(0)
                        .width(Length::Fill)
                        .on_press(Msg::GoTo(k.page, k.block)),
                        link("×", Msg::DeleteBookmark(k.id), alpha(CREAM, 0.6)),
                    ]
                    .spacing(8),
                );
            }
            col = col.push(Space::with_height(6)).push(
                row![
                    text("Highlights").size(15).font(BLACK),
                    horizontal_space(),
                    link("copy all as Markdown", Msg::Export, alpha(CREAM, 0.75)),
                ]
                .align_y(Alignment::End),
            );
            if o.full.highlights.is_empty() {
                col = col.push(mono(
                    if epub {
                        "click a word, then the last word, then a colour"
                    } else {
                        "drag over the words, then a colour"
                    },
                    12.0,
                    alpha(CREAM, 0.7),
                ));
            }
            for h in &o.full.highlights {
                let place = if epub {
                    format!("ch. {}", h.page + 1)
                } else {
                    format!("p. {}", h.page + 1)
                };
                let block = match h.anchor {
                    Anchor::Text { b0, .. } => b0,
                    _ => 0,
                };
                let mut body =
                    column![text(h.text.clone()).size(14).font(SERIF).line_height(1.4)].spacing(3);
                if !h.note.is_empty() {
                    body = body.push(mono(h.note.clone(), 12.0, CREAM));
                }
                let (page, id) = (h.page, h.id);
                body = body.push(row![
                    mono(place, 11.0, alpha(CREAM, 0.7)),
                    horizontal_space(),
                    link(
                        if h.note.is_empty() {
                            "add a note"
                        } else {
                            "edit note"
                        },
                        Msg::OpenNote(id),
                        alpha(CREAM, 0.7)
                    ),
                ]);
                col = col.push(
                    mouse_area(
                        row![
                            container(Space::new(Length::Fixed(3.0), Length::Fixed(40.0)))
                                .style(style::filled(mark_color(&h.color))),
                            body
                        ]
                        .spacing(10),
                    )
                    .on_press(Msg::GoTo(page, block))
                    .on_right_press(Msg::OpenNote(id))
                    .interaction(mouse::Interaction::Pointer),
                );
            }
        }
        container(
            scrollable(col.padding(iced::Padding {
                right: 12.0,
                ..Default::default()
            }))
            .style(style::scroll(alpha(CREAM, 0.5))),
        )
        .padding([20, 20])
        .width(Length::Fixed(SIDE_W))
        .height(Length::Fill)
        .into()
    }

    // ---- settings -----------------------------------------------------------------

    pub fn settings_view<'a>(&'a self, library: &str, note: Option<&'a str>) -> Element<'a, Msg> {
        let ink = BOOKS_INK;
        let line = |name: &str, value: String, color: Color| -> Element<'a, Msg> {
            column![
                row![
                    text(name.to_string()).size(15).color(color),
                    horizontal_space(),
                    mono(value, 13.0, color)
                ]
                .align_y(Alignment::End),
                Space::with_height(6),
                horizontal_rule(1).style(style::hairline(alpha(color, 0.3))),
            ]
            .into()
        };
        let root = library
            .trim_end_matches("/Music")
            .trim_end_matches("Music")
            .to_string();
        let books_n = self.books.iter().filter(|b| !b.is_comic()).count();
        let comics_n = self.books.len() - books_n;
        let p = &self.prefs;
        let opt = |label: String, on: bool, m: Msg, c: Color| -> Element<'a, Msg> {
            button(text(label).size(12).font(MONO))
                .style(move |_t: &Theme, s: button::Status| {
                    let fill = on || matches!(s, button::Status::Hovered);
                    button::Style {
                        background: Some(if fill { c } else { Color::TRANSPARENT }.into()),
                        text_color: if fill {
                            if c == ink {
                                BOOKS
                            } else {
                                COMICS
                            }
                        } else {
                            c
                        },
                        border: iced::Border {
                            color: c,
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        shadow: Default::default(),
                    }
                })
                .padding([7, 10])
                .on_press(m)
                .into()
        };
        let books =
            column![
                row![
                    text("Books").size(30).font(BLACK).color(ink),
                    Space::with_width(14),
                    mono(format!("{books_n} books · EPUB and PDF"), 12.0, ink)
                ]
                .align_y(Alignment::End),
                horizontal_rule(1).style(style::hairline(ink)),
                line("Folder", format!("{}/Books", tilde(&root)), ink),
                line(
                    "Pages",
                    if p.dark {
                        "dark".into()
                    } else {
                        "paper".into()
                    },
                    ink
                ),
                line(
                    "PDF pages",
                    match p.pdf_fit {
                        Fit::Width => "fit the width",
                        Fit::Page => "a whole page",
                        Fit::Two => "two pages",
                    }
                    .into(),
                    ink
                ),
                line("Covers and details", "from the file".into(), ink),
                Space::with_height(8),
                text("Reading font").size(20).font(BLACK).color(ink),
                container(
                    column![
                text("Literata, made for long reading on screens. It has the č, ć, đ, š and ž.")
                    .size(p.size)
                    .font(SERIF)
                    .line_height(p.line)
                    .color(ink),
                row![
                    opt("A−".into(), false, Msg::Size(-1.0), ink),
                    opt(format!("size {}", p.size), true, Msg::Size(0.0), ink),
                    opt("A+".into(), false, Msg::Size(1.0), ink),
                    Space::with_width(8),
                    opt("line 1.5".into(), (p.line - 1.5).abs() < 0.01, Msg::Line(1.5), ink),
                    opt("1.65".into(), (p.line - 1.65).abs() < 0.01, Msg::Line(1.65), ink),
                    opt("1.8".into(), (p.line - 1.8).abs() < 0.01, Msg::Line(1.8), ink),
                ]
                .spacing(6),
                row![
                    opt("column 54 letters".into(), p.measure == 54.0, Msg::Measure(54.0), ink),
                    opt("62".into(), p.measure == 62.0, Msg::Measure(62.0), ink),
                    opt("72".into(), p.measure == 72.0, Msg::Measure(72.0), ink),
                    Space::with_width(8),
                    opt("Dark".into(), p.dark, Msg::Dark, ink),
                    opt("Paper".into(), !p.dark, Msg::Dark, ink),
                ]
                .spacing(6),
            ]
                    .spacing(12)
                )
                .padding(16)
                .style(style::outlined(ink)),
                Space::with_height(4),
                mono(
                    note.unwrap_or(
                        "Highlights and notes: open a book, Marks, copy all as Markdown."
                    ),
                    12.0,
                    ink
                ),
            ]
            .spacing(12);
        let c = CREAM;
        let comics = column![
            row![
                text("Comics").size(30).font(BLACK),
                Space::with_width(14),
                mono(
                    format!("{comics_n} issues · CBR, CBZ, PDF, folders of pages"),
                    12.0,
                    c
                )
            ]
            .align_y(Alignment::End),
            horizontal_rule(1).style(style::hairline(c)),
            line("Folder", format!("{}/Comics", tilde(&root)), c),
            line("Series", "one folder a series, issues by number".into(), c),
            line("CBR", "needs bsdtar or unrar".into(), c),
            row![
                text("On a wide window").size(15),
                horizontal_space(),
                opt("spread, cover alone".into(), p.spread, Msg::Spread, c),
                opt("one page".into(), !p.spread, Msg::Spread, c)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![
                text("Reading direction").size(15),
                horizontal_space(),
                opt("left to right".into(), !p.right_to_left, Msg::Direction, c),
                opt("right to left".into(), p.right_to_left, Msg::Direction, c)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            Space::with_height(8),
            text("Rotate").size(20).font(BLACK),
            row![
                text("Rotate turns").size(15),
                horizontal_space(),
                opt(
                    "the whole screen".into(),
                    p.rotate_screen,
                    Msg::RotateScreen,
                    c
                ),
                opt(
                    "the page only".into(),
                    !p.rotate_screen,
                    Msg::RotateScreen,
                    c
                )
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            line(
                "Screen",
                match &self.output {
                    Some(Some(o)) => format!("{o} · back to normal when the reader closes"),
                    Some(None) => "no xrandr here (Wayland?): the page turns".into(),
                    None => "xrandr on X11 · back to normal when the reader closes".into(),
                },
                c,
            ),
        ]
        .spacing(12);
        row![
            container(scrollable(books).style(style::scroll(ink)))
                .padding([32, 40])
                .style(style::filled_text(BOOKS, ink))
                .width(Length::FillPortion(1))
                .height(Length::Fill),
            container(scrollable(comics).style(style::scroll(c)))
                .padding([32, 40])
                .style(style::filled_text(COMICS, c))
                .width(Length::FillPortion(1))
                .height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && path.starts_with(&h) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

/// The line along the foot: how far through, and the bookmarks on it.
/// The reading line in the foot: how far along, with a tick for each bookmark.
/// Plain widgets rather than a canvas, so it draws over page images too.
fn progress_line<'a>(at: f32, mut ticks: Vec<f32>) -> Element<'a, Msg> {
    let part = |n: f32| ((n.clamp(0.0, 1.0) * 1000.0).round() as u16).max(1);
    // The width goes on the container itself: a container only passes on Fill,
    // not its portion.
    let bar = |c: Color, w: Length| {
        container(Space::new(Length::Fill, Length::Fixed(2.0)))
            .width(w)
            .style(style::filled(c))
    };
    let done = row![
        bar(CREAM, Length::FillPortion(part(at))),
        bar(alpha(CREAM, 0.25), Length::FillPortion(part(1.0 - at))),
    ];
    ticks.sort_by(|a, b| a.total_cmp(b));
    let mut marks = row![].height(Length::Fixed(12.0));
    let mut last = 0.0;
    for t in ticks {
        marks = marks
            .push(Space::new(
                Length::FillPortion(part(t - last)),
                Length::Fixed(12.0),
            ))
            .push(
                container(Space::new(Length::Fixed(2.0), Length::Fixed(12.0)))
                    .style(style::filled(style::OCHRE)),
            );
        last = t;
    }
    marks = marks.push(Space::new(
        Length::FillPortion(part(1.0 - last)),
        Length::Fixed(12.0),
    ));
    iced::widget::stack![
        container(done)
            .center_y(Length::Fixed(12.0))
            .width(Length::Fill),
        marks
    ]
    .width(Length::Fill)
    .height(Length::Fixed(12.0))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(spans: &[(&str, bool)]) -> Block {
        Block {
            kind: "p".into(),
            spans: spans
                .iter()
                .map(|(t, i)| Span {
                    text: t.to_string(),
                    italic: *i,
                    bold: false,
                })
                .collect(),
            src: None,
        }
    }

    #[test]
    fn words_keep_their_spaces_and_styles() {
        let t = tokens(&block(&[
            ("Someone must ", false),
            ("Josef K", true),
            (". He", false),
        ]));
        let words: Vec<(usize, &str)> = t.iter().map(|t| (t.word, t.text.as_str())).collect();
        assert_eq!(
            words,
            [
                (0, "Someone "),
                (1, "must "),
                (2, "Josef "),
                (3, "K"),
                (3, ". "),
                (4, "He")
            ]
        );
        assert!(t[2].italic && !t[4].italic);
    }

    #[test]
    fn page_words_join_into_one_box_a_line() {
        let w = |x: f32, line: u32| Word {
            text: "w".into(),
            x,
            y: line as f32 * 0.1,
            w: 0.1,
            h: 0.05,
            line,
        };
        let words = [w(0.1, 0), w(0.3, 0), w(0.1, 1)];
        let r = rects(&words, 0, 2);
        assert_eq!(r.len(), 2);
        assert!((r[0][2] - 0.3).abs() < 1e-6);
        assert_eq!(word_at(&words, Point::new(0.35, 0.02)), Some(1));
        assert_eq!(word_at(&words, Point::new(0.9, 0.9)), None);
    }

    #[test]
    fn a_comic_spread_keeps_the_cover_alone() {
        let mut r = Reader {
            window: Size::new(1600.0, 900.0),
            ..Default::default()
        };
        r.prefs.spread = true;
        r.prefs.right_to_left = false;
        let mut o = open_with(10);
        o.page = 0;
        assert_eq!(r.shown_pages(&o), [0]);
        o.page = 2;
        assert_eq!(r.shown_pages(&o), [1, 2]);
        o.page = 3;
        assert_eq!(r.shown_pages(&o), [3, 4]);
        r.prefs.right_to_left = true;
        assert_eq!(r.shown_pages(&o), [4, 3]);
        r.window = Size::new(900.0, 1600.0);
        assert_eq!(r.shown_pages(&o), [3]);
    }

    fn open_with(pages: i64) -> Open {
        Open {
            full: Full {
                book: Book {
                    kind: "comic".into(),
                    format: "cbz".into(),
                    pages,
                    ..Default::default()
                },
                chapters: vec![],
                bookmarks: vec![],
                highlights: vec![],
            },
            page: 0,
            block: 0,
            chapter: None,
            toks: vec![],
            pics: HashMap::new(),
            loading: HashSet::new(),
            res: HashMap::new(),
            words: HashMap::new(),
            sel: None,
            side: Side::None,
            editing: None,
            note: text_editor::Content::new(),
            goto: String::new(),
            rel: 0.0,
            saved_at: Instant::now(),
            chrome: true,
            flash: None,
        }
    }
}
