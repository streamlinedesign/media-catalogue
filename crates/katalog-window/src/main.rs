//! katalog-window: the desktop view of katalogd.
//!
//! One window, drawn by iced, in the Kaptol design: Satoshi, navy and cream,
//! headings on rules, numbers pinned to the right, a black card for what is
//! playing. Everything it shows comes from katalogd over HTTP; it holds no
//! library state of its own.
//!
//!   katalog-window                       talks to http://127.0.0.1:7300
//!   katalog-window --url http://host:7300
//!   space: play/pause   n / p: next, previous   r: radio   esc: back

mod api;
mod icons;
mod style;

use api::{
    fmt_long, fmt_ms, Album, AlbumWithTracks, Artist, Client, Lyrics, LyricsHit, PlayState, Player,
    Playlist, PlaylistWithTracks, Queue, Status, Track,
};
use clap::Parser;
use iced::keyboard::{self, key::Named, Key};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, image, mouse_area, row,
    scrollable, text, text_input, vertical_space, Space,
};
use iced::{window, Alignment, Element, Length, Size, Subscription, Task};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use style::{
    alpha, BLACK, BOLD, CARD, CARD_LINE, CARD_LINE_STRONG, CARD_MUTE, CARD_SOFT, CARD_TEXT, CREAM,
    MEDIUM, RED, WHITE,
};

#[derive(Parser, Debug)]
#[command(version, about = "The Katalog window")]
struct Args {
    /// Where katalogd listens.
    #[arg(long, env = "KATALOG_URL", default_value = "http://127.0.0.1:7300")]
    url: String,
}

fn main() -> iced::Result {
    let args = Args::parse();
    iced::application("Katalog", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| style::theme())
        .font(style::SATOSHI_REGULAR)
        .font(style::SATOSHI_MEDIUM)
        .font(style::SATOSHI_BOLD)
        .font(style::SATOSHI_BLACK)
        .default_font(MEDIUM)
        .antialiasing(true)
        .window_size(Size::new(1280.0, 840.0))
        .run_with(move || App::new(args.url.clone()))
}

const GUTTER: f32 = 32.0;
const CARD_MIN: f32 = 200.0;
const CARD_GAP: f32 = 24.0;
const ROW_GAP: f32 = 32.0;
/// Below the cover: title row, rule, artist line.
const CARD_TEXT_H: f32 = 70.0;
const THUMB: u32 = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Albums,
    Artists,
    Playlists,
    Queue,
    Lyrics,
}

#[derive(Debug, Clone, PartialEq)]
enum View {
    Grid,
    Album(i64),
}

/// What the Playlists tab shows.
#[derive(Debug, Clone, PartialEq)]
enum ListView {
    All,
    One(i64),
}

struct App {
    client: Client,
    status: Status,
    albums: Vec<Album>,
    artists: Vec<Artist>,
    open_album: Option<AlbumWithTracks>,
    playlists: Vec<Playlist>,
    open_playlist: Option<PlaylistWithTracks>,
    list_view: ListView,
    new_playlist: String,
    /// The track whose "add to playlist" chooser is open, and where it already is.
    adding: Option<i64>,
    adding_in: Vec<i64>,
    query: String,
    artist_filter: Option<i64>,
    tab: Tab,
    view: View,
    player: Player,
    queue: Option<Queue>,
    lyrics: Option<Lyrics>,
    lyrics_for: Option<i64>,
    lyrics_line: Option<usize>,
    /// The lyrics search, for the playing track: the query, whether the
    /// artist narrows it, the hits, and what is going on.
    lyrics_query: String,
    lyrics_with_artist: bool,
    lyrics_hits: Vec<LyricsHit>,
    lyrics_searching: bool,
    lyrics_note: Option<String>,
    art: HashMap<(i64, u32), image::Handle>,
    art_loading: HashSet<(i64, u32)>,
    width: f32,
    grid_scroll: f32,
    grid_height: f32,
    error: Option<String>,
    ticks: u64,
    screenshot: Option<String>,
    fake: bool,
    fake_nolyrics: bool,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum Message {
    Tick,
    Status(Result<Status, String>),
    Albums(Result<Vec<Album>, String>),
    Artists(Result<Vec<Artist>, String>),
    AlbumLoaded(Result<AlbumWithTracks, String>),
    Playlists(Result<Vec<Playlist>, String>),
    PlaylistLoaded(Result<PlaylistWithTracks, String>),
    OpenPlaylist(i64),
    NewPlaylistName(String),
    CreatePlaylist,
    /// Open or close the chooser under a track.
    AddTo(i64),
    AddingIn(i64, Result<Vec<i64>, String>),
    AddToPlaylist(i64, i64),
    /// Make a playlist from the chooser's text and put the track in it.
    CreateAndAdd(i64),
    PlaylistChanged(Result<PlaylistWithTracks, String>),
    RemoveFromPlaylist(i64, usize),
    DeletePlaylist(i64),
    PlaylistDeleted(Result<serde_json::Value, String>),
    PlayPlaylist(i64, usize),
    AddPlaylist(i64),
    RadioOn(Option<i64>),
    RadioOff,
    PlayerState(Result<Player, String>),
    QueueLoaded(Result<Queue, String>),
    LyricsLoaded(i64, Result<Lyrics, String>),
    LyricsQuery(String),
    /// Search LRCLIB for the playing track; `true` keeps the artist in.
    LyricsSearch(bool),
    LyricsHits(i64, Result<Vec<LyricsHit>, String>),
    LyricsPick(usize),
    LyricsSaved(i64, Result<Lyrics, String>),
    Art(i64, u32, Result<Vec<u8>, String>),
    Search(String),
    Tab(Tab),
    OpenAlbum(i64),
    OpenArtist(i64),
    Back,
    PlayAlbum(i64, usize),
    AddAlbum(i64),
    Toggle,
    Next,
    Prev,
    SeekFraction(f32),
    SeekMs(u64),
    Jump(usize),
    GridScrolled(scrollable::Viewport),
    Resized(f32, f32),
    DebugFake,
    DebugNoLyrics,
    Shot(window::Screenshot),
}

impl App {
    fn new(url: String) -> (Self, Task<Message>) {
        let client = Client::new(url);
        let app = Self {
            client: client.clone(),
            status: Status::default(),
            albums: Vec::new(),
            artists: Vec::new(),
            open_album: None,
            playlists: Vec::new(),
            open_playlist: None,
            list_view: ListView::All,
            new_playlist: String::new(),
            adding: None,
            adding_in: Vec::new(),
            query: String::new(),
            artist_filter: None,
            tab: Tab::Albums,
            view: View::Grid,
            player: Player::default(),
            queue: None,
            lyrics: None,
            lyrics_for: None,
            lyrics_line: None,
            lyrics_query: String::new(),
            lyrics_with_artist: true,
            lyrics_hits: Vec::new(),
            lyrics_searching: false,
            lyrics_note: None,
            art: HashMap::new(),
            art_loading: HashSet::new(),
            width: 1280.0,
            grid_scroll: 0.0,
            grid_height: 600.0,
            error: None,
            ticks: 0,
            screenshot: std::env::var("KATALOG_SCREENSHOT").ok(),
            fake: false,
            fake_nolyrics: false,
        };
        let mut tasks = vec![
            app.load_status(),
            app.load_albums(),
            app.load_artists(),
            app.load_player(),
            app.load_playlists(),
            text_input::focus(text_input::Id::new("search")),
        ];
        // For screenshots in development:
        // KATALOG_DEBUG_VIEW=album:3 | play:3 | playlist:1 | adding:5 | queue | lyrics | artists | playlists | fake
        if let Ok(v) = std::env::var("KATALOG_DEBUG_VIEW") {
            for part in v.split(',') {
                let msg = match part.split_once(':') {
                    Some(("album", id)) => id.parse().ok().map(Message::OpenAlbum),
                    Some(("play", id)) => id.parse().ok().map(|id| Message::PlayAlbum(id, 0)),
                    Some(("playlist", id)) => id.parse().ok().map(Message::OpenPlaylist),
                    Some(("adding", id)) => id.parse().ok().map(Message::AddTo),
                    _ => match part {
                        "playlists" => Some(Message::Tab(Tab::Playlists)),
                        "queue" => Some(Message::Tab(Tab::Queue)),
                        "lyrics" => Some(Message::Tab(Tab::Lyrics)),
                        "artists" => Some(Message::Tab(Tab::Artists)),
                        "fake" => Some(Message::DebugFake),
                        "search" => Some(Message::LyricsSearch(true)),
                        "nolyrics" => Some(Message::DebugNoLyrics),
                        "fakehits" => Some(Message::LyricsHits(
                            0,
                            Ok(vec![
                                LyricsHit {
                                    source: "lrclib".into(),
                                    title: "Paranoid Android".into(),
                                    artist: "Radiohead".into(),
                                    album: "OK Computer".into(),
                                    duration_ms: Some(383_000),
                                    synced: true,
                                    text: "[00:14.20]Please could you stop the noise\n[00:17.00]I'm trying to get some rest".into(),
                                },
                                LyricsHit {
                                    source: "lrclib".into(),
                                    title: "Paranoid Android (Live)".into(),
                                    artist: "Radiohead".into(),
                                    album: "I Might Be Wrong".into(),
                                    duration_ms: Some(401_000),
                                    synced: false,
                                    text: "Please could you stop the noise\nI'm trying to get some rest".into(),
                                },
                            ]),
                        )),
                        _ => None,
                    },
                };
                if let Some(m) = msg {
                    tasks.push(Task::done(m));
                }
            }
        }
        let task = Task::batch(tasks);
        let _ = client;
        (app, task)
    }

    fn load_status(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(async move { c.get("/api/status").await }, Message::Status)
    }
    fn load_albums(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(async move { c.get("/api/albums").await }, Message::Albums)
    }
    fn load_artists(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(async move { c.get("/api/artists").await }, Message::Artists)
    }
    fn load_player(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get("/api/player").await },
            Message::PlayerState,
        )
    }
    fn load_queue(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get("/api/player/queue").await },
            Message::QueueLoaded,
        )
    }
    fn load_lyrics(&self, track_id: i64) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get(&format!("/api/tracks/{track_id}/lyrics")).await },
            move |r| Message::LyricsLoaded(track_id, r),
        )
    }
    fn load_playlists(&self) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get("/api/playlists").await },
            Message::Playlists,
        )
    }
    fn load_playlist(&self, id: i64) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get(&format!("/api/playlists/{id}")).await },
            Message::PlaylistLoaded,
        )
    }
    fn playlist_post(&self, path: &str, body: serde_json::Value) -> Task<Message> {
        let c = self.client.clone();
        let path = path.to_string();
        Task::perform(
            async move { c.post::<PlaylistWithTracks>(&path, Some(body)).await },
            Message::PlaylistChanged,
        )
    }
    fn load_album(&self, id: i64) -> Task<Message> {
        let c = self.client.clone();
        Task::perform(
            async move { c.get(&format!("/api/albums/{id}")).await },
            Message::AlbumLoaded,
        )
    }
    fn player_post(&self, path: &str, body: Option<serde_json::Value>) -> Task<Message> {
        let c = self.client.clone();
        let path = path.to_string();
        Task::perform(
            async move { c.post::<Player>(&path, body).await },
            Message::PlayerState,
        )
    }

    /// Asks for a cover once; later ticks find it loaded or on its way.
    fn want_art(&mut self, art_id: i64, size: u32) -> Task<Message> {
        let key = (art_id, size);
        if self.art.contains_key(&key) || self.art_loading.contains(&key) {
            return Task::none();
        }
        self.art_loading.insert(key);
        let c = self.client.clone();
        Task::perform(
            async move { c.bytes(&format!("/art/{art_id}?size={size}&crop=1")).await },
            move |r| Message::Art(art_id, size, r),
        )
    }

    fn columns(&self) -> usize {
        let inner = (self.width - 2.0 * GUTTER + CARD_GAP).max(CARD_MIN);
        ((inner / (CARD_MIN + CARD_GAP)).floor() as usize).clamp(2, 8)
    }
    fn card_width(&self) -> f32 {
        let cols = self.columns() as f32;
        ((self.width - 2.0 * GUTTER) - CARD_GAP * (cols - 1.0)) / cols
    }

    fn filtered_albums(&self) -> Vec<&Album> {
        let q = self.query.trim().to_lowercase();
        self.albums
            .iter()
            .filter(|a| {
                self.artist_filter
                    .map(|id| a.artist_id == id)
                    .unwrap_or(true)
            })
            .filter(|a| {
                q.is_empty()
                    || a.title.to_lowercase().contains(&q)
                    || a.artist.to_lowercase().contains(&q)
                    || a.year.map(|y| year_matches(y, &q)).unwrap_or(false)
            })
            .collect()
    }

    fn filtered_artists(&self) -> Vec<&Artist> {
        let q = self.query.trim().to_lowercase();
        self.artists
            .iter()
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q))
            .collect()
    }

    /// Covers for the albums in view, plus a row above and below.
    fn load_visible_art(&mut self) -> Task<Message> {
        if self.tab != Tab::Albums || self.view != View::Grid {
            return Task::none();
        }
        let cols = self.columns();
        let row_h = self.card_width() + CARD_TEXT_H + ROW_GAP;
        let first_row = ((self.grid_scroll / row_h).floor() as usize).saturating_sub(1);
        let last_row = ((self.grid_scroll + self.grid_height) / row_h).ceil() as usize + 1;
        let ids: Vec<i64> = self
            .filtered_albums()
            .iter()
            .skip(first_row * cols)
            .take((last_row - first_row + 1) * cols)
            .filter_map(|a| a.art_id)
            .collect();
        Task::batch(ids.into_iter().map(|id| self.want_art(id, THUMB)))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.ticks += 1;
                let mut tasks = if self.fake {
                    Vec::new()
                } else {
                    vec![self.load_player()]
                };
                if self.ticks.is_multiple_of(20) {
                    tasks.push(self.load_status());
                    tasks.push(self.load_playlists());
                }
                tasks.push(self.load_visible_art());
                if self.ticks == 10 && self.screenshot.is_some() {
                    tasks.push(
                        window::get_latest()
                            .and_then(window::screenshot)
                            .map(Message::Shot),
                    );
                }
                Task::batch(tasks)
            }
            Message::Status(Ok(s)) => {
                let changed = s.albums != self.status.albums || s.tracks != self.status.tracks;
                self.status = s;
                self.error = None;
                if changed {
                    Task::batch([self.load_albums(), self.load_artists()])
                } else {
                    Task::none()
                }
            }
            Message::Status(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::Albums(Ok(a)) => {
                self.albums = a;
                self.error = None;
                self.load_visible_art()
            }
            Message::Artists(Ok(a)) => {
                self.artists = a;
                Task::none()
            }
            Message::Albums(Err(e)) | Message::Artists(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::AlbumLoaded(Ok(a)) => {
                let art = a.album.art_id;
                self.open_album = Some(a);
                let mut tasks = Vec::new();
                if let Some(id) = art {
                    tasks.push(self.want_art(id, 640));
                    tasks.push(self.want_art(id, 128));
                }
                if self.fake {
                    tasks.push(Task::done(Message::DebugFake));
                }
                Task::batch(tasks)
            }
            Message::AlbumLoaded(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::Playlists(Ok(p)) => {
                self.playlists = p;
                Task::none()
            }
            Message::Playlists(Err(_)) => Task::none(),
            Message::PlaylistLoaded(Ok(p)) => {
                let art: Vec<i64> = p.tracks.iter().filter_map(|t| t.art_id).collect();
                self.open_playlist = Some(p);
                Task::batch(art.into_iter().map(|id| self.want_art(id, 128)))
            }
            Message::PlaylistLoaded(Err(e)) => {
                self.error = Some(e);
                self.list_view = ListView::All;
                Task::none()
            }
            Message::OpenPlaylist(id) => {
                self.tab = Tab::Playlists;
                self.list_view = ListView::One(id);
                self.adding = None;
                if self.open_playlist.as_ref().map(|p| p.playlist.id) != Some(id) {
                    self.open_playlist = None;
                }
                self.load_playlist(id)
            }
            Message::NewPlaylistName(n) => {
                self.new_playlist = n;
                Task::none()
            }
            Message::CreatePlaylist => {
                let name = self.new_playlist.trim().to_string();
                if name.is_empty() {
                    return Task::none();
                }
                self.new_playlist.clear();
                let c = self.client.clone();
                Task::perform(
                    async move {
                        c.post::<Playlist>(
                            "/api/playlists",
                            Some(serde_json::json!({ "name": name })),
                        )
                        .await
                    },
                    |r| match r {
                        Ok(p) => Message::OpenPlaylist(p.id),
                        Err(e) => Message::Playlists(Err(e)),
                    },
                )
            }
            Message::AddTo(track_id) => {
                if self.adding == Some(track_id) {
                    self.adding = None;
                    return Task::none();
                }
                self.adding = Some(track_id);
                self.adding_in.clear();
                self.new_playlist.clear();
                let c = self.client.clone();
                Task::batch([
                    Task::perform(
                        async move { c.get(&format!("/api/tracks/{track_id}/playlists")).await },
                        move |r| Message::AddingIn(track_id, r),
                    ),
                    self.load_playlists(),
                ])
            }
            Message::AddingIn(track_id, Ok(ids)) => {
                if self.adding == Some(track_id) {
                    self.adding_in = ids;
                }
                Task::none()
            }
            Message::AddingIn(_, Err(_)) => Task::none(),
            Message::AddToPlaylist(playlist_id, track_id) => {
                if !self.adding_in.contains(&playlist_id) {
                    self.adding_in.push(playlist_id);
                }
                self.playlist_post(
                    &format!("/api/playlists/{playlist_id}/tracks"),
                    serde_json::json!({ "tracks": [track_id] }),
                )
            }
            Message::CreateAndAdd(track_id) => {
                let name = self.new_playlist.trim().to_string();
                if name.is_empty() {
                    return Task::none();
                }
                self.new_playlist.clear();
                let c = self.client.clone();
                Task::perform(
                    async move {
                        let p = c
                            .post::<Playlist>(
                                "/api/playlists",
                                Some(serde_json::json!({ "name": name })),
                            )
                            .await?;
                        c.post::<PlaylistWithTracks>(
                            &format!("/api/playlists/{}/tracks", p.id),
                            Some(serde_json::json!({ "tracks": [track_id] })),
                        )
                        .await
                    },
                    Message::PlaylistChanged,
                )
            }
            Message::PlaylistChanged(Ok(p)) => {
                if let Some(t) = self.adding {
                    if p.tracks.iter().any(|x| x.id == t)
                        && !self.adding_in.contains(&p.playlist.id)
                    {
                        self.adding_in.push(p.playlist.id);
                    }
                }
                if self.open_playlist.as_ref().map(|o| o.playlist.id) == Some(p.playlist.id) {
                    self.open_playlist = Some(p);
                }
                self.load_playlists()
            }
            Message::PlaylistChanged(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::RemoveFromPlaylist(id, pos) => {
                let c = self.client.clone();
                Task::perform(
                    async move {
                        c.delete::<PlaylistWithTracks>(&format!("/api/playlists/{id}/tracks/{pos}"))
                            .await
                    },
                    Message::PlaylistChanged,
                )
            }
            Message::DeletePlaylist(id) => {
                let c = self.client.clone();
                self.list_view = ListView::All;
                self.open_playlist = None;
                Task::perform(
                    async move {
                        c.delete::<serde_json::Value>(&format!("/api/playlists/{id}"))
                            .await
                    },
                    Message::PlaylistDeleted,
                )
            }
            Message::PlaylistDeleted(Ok(_)) => self.load_playlists(),
            Message::PlaylistDeleted(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::PlayPlaylist(id, start) => self.player_post(
                "/api/player/play",
                Some(serde_json::json!({ "playlist": id, "start": start })),
            ),
            Message::AddPlaylist(id) => self.player_post(
                "/api/player/queue",
                Some(serde_json::json!({ "playlist": id })),
            ),
            Message::RadioOn(playlist) => self.player_post(
                "/api/radio",
                Some(serde_json::json!({ "playlist": playlist })),
            ),
            Message::RadioOff => {
                let c = self.client.clone();
                Task::perform(
                    async move { c.delete::<Player>("/api/radio").await },
                    Message::PlayerState,
                )
            }
            Message::PlayerState(Ok(p)) => {
                let track_changed =
                    p.track.as_ref().map(|t| t.id) != self.player.track.as_ref().map(|t| t.id);
                let queue_changed = p.queue != self.player.queue;
                self.player = p;
                self.error = None;
                let mut tasks = Vec::new();
                if let Some(t) = self.player.track.clone() {
                    if let Some(a) = t.art_id {
                        tasks.push(self.want_art(a, 128));
                    }
                    if track_changed || self.lyrics_for != Some(t.id) {
                        self.lyrics = None;
                        self.lyrics_line = None;
                        self.lyrics_for = Some(t.id);
                        self.lyrics_query = t.title.clone();
                        self.lyrics_with_artist = true;
                        self.lyrics_hits.clear();
                        self.lyrics_searching = false;
                        self.lyrics_note = None;
                        if t.has_lyrics {
                            tasks.push(self.load_lyrics(t.id));
                        }
                    }
                } else {
                    self.lyrics = None;
                    self.lyrics_for = None;
                    self.lyrics_line = None;
                }
                if queue_changed || (self.queue.is_none() && !self.player.queue.is_empty()) {
                    tasks.push(self.load_queue());
                }
                // Follow the synced lyrics.
                if let Some(l) = &self.lyrics {
                    if l.synced {
                        let pos = self.player.position_ms;
                        let lines = l.lines();
                        let idx = lines
                            .iter()
                            .enumerate()
                            .filter(|(_, ln)| ln.at_ms.map(|t| t <= pos).unwrap_or(false))
                            .map(|(i, _)| i)
                            .next_back();
                        if idx != self.lyrics_line {
                            self.lyrics_line = idx;
                            if self.tab == Tab::Lyrics {
                                if let Some(i) = idx {
                                    let y = (i as f32 * LYRIC_LINE_H - 130.0).max(0.0);
                                    tasks.push(scrollable::scroll_to(
                                        scrollable::Id::new("lyrics"),
                                        scrollable::AbsoluteOffset { x: 0.0, y },
                                    ));
                                }
                            }
                        }
                    }
                }
                Task::batch(tasks)
            }
            Message::PlayerState(Err(e)) => {
                self.error = Some(e);
                Task::none()
            }
            Message::QueueLoaded(Ok(q)) => {
                self.queue = Some(q);
                Task::none()
            }
            Message::QueueLoaded(Err(_)) => Task::none(),
            Message::LyricsLoaded(id, Ok(l)) => {
                if self.lyrics_for == Some(id) {
                    self.lyrics = Some(l);
                }
                Task::none()
            }
            Message::LyricsLoaded(_, Err(_)) => Task::none(),
            Message::LyricsQuery(q) => {
                self.lyrics_query = q;
                Task::none()
            }
            Message::LyricsSearch(with_artist) => {
                let Some(t) = self.player.track.clone() else {
                    return Task::none();
                };
                let q = self.lyrics_query.trim().to_string();
                if q.is_empty() {
                    return Task::none();
                }
                self.lyrics_with_artist = with_artist;
                self.lyrics_searching = true;
                self.lyrics_note = None;
                let mut path = format!("/api/tracks/{}/lyrics/search?q={}", t.id, urlencode(&q));
                if with_artist {
                    path.push_str(&format!("&artist={}", urlencode(&t.artist)));
                }
                let c = self.client.clone();
                Task::perform(
                    async move { c.get::<Vec<LyricsHit>>(&path).await },
                    move |r| Message::LyricsHits(t.id, r),
                )
            }
            Message::LyricsHits(id, r) => {
                if id != 0 && self.lyrics_for != Some(id) {
                    return Task::none();
                }
                self.lyrics_searching = false;
                match r {
                    Ok(hits) => {
                        self.lyrics_note = if hits.is_empty() {
                            Some("Nothing found. Try fewer words, or without the artist.".into())
                        } else {
                            None
                        };
                        self.lyrics_hits = hits;
                    }
                    Err(e) => {
                        self.lyrics_hits.clear();
                        self.lyrics_note = Some(e);
                    }
                }
                Task::none()
            }
            Message::LyricsPick(i) => {
                let (Some(t), Some(hit)) = (self.player.track.as_ref(), self.lyrics_hits.get(i))
                else {
                    return Task::none();
                };
                let id = t.id;
                let body = serde_json::json!({ "text": hit.text, "source": hit.source });
                self.lyrics_note = Some("Saving…".into());
                let c = self.client.clone();
                Task::perform(
                    async move {
                        c.put::<Lyrics>(&format!("/api/tracks/{id}/lyrics"), body)
                            .await
                    },
                    move |r| Message::LyricsSaved(id, r),
                )
            }
            Message::LyricsSaved(id, Ok(l)) => {
                if self.lyrics_for == Some(id) {
                    self.lyrics = Some(l);
                    self.lyrics_hits.clear();
                    self.lyrics_note = None;
                    if let Some(t) = self.player.track.as_mut() {
                        t.has_lyrics = true;
                    }
                }
                Task::none()
            }
            Message::LyricsSaved(_, Err(e)) => {
                self.lyrics_note = Some(e);
                Task::none()
            }
            Message::Art(id, size, Ok(bytes)) => {
                self.art_loading.remove(&(id, size));
                self.art
                    .insert((id, size), image::Handle::from_bytes(bytes));
                Task::none()
            }
            Message::Art(id, size, Err(_)) => {
                self.art_loading.remove(&(id, size));
                Task::none()
            }
            Message::Search(q) => {
                self.query = q;
                self.view = View::Grid;
                if self.tab == Tab::Queue || self.tab == Tab::Lyrics || self.tab == Tab::Playlists {
                    self.tab = Tab::Albums;
                }
                self.grid_scroll = 0.0;
                Task::batch([
                    scrollable::scroll_to(
                        scrollable::Id::new("grid"),
                        scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
                    ),
                    self.load_visible_art(),
                ])
            }
            Message::Tab(t) => {
                if t == Tab::Albums && self.tab == Tab::Albums && self.view != View::Grid {
                    // Already on Albums, looking at one album: the tab is the way back.
                    self.view = View::Grid;
                }
                if t == Tab::Playlists && self.tab == Tab::Playlists {
                    self.list_view = ListView::All;
                }
                self.adding = None;
                self.tab = t;
                if t == Tab::Playlists {
                    return self.load_playlists();
                }
                if t == Tab::Queue {
                    return self.load_queue();
                }
                self.load_visible_art()
            }
            Message::OpenAlbum(id) => {
                self.view = View::Album(id);
                self.tab = Tab::Albums;
                self.open_album = None;
                self.load_album(id)
            }
            Message::OpenArtist(id) => {
                self.artist_filter = Some(id);
                self.query.clear();
                self.tab = Tab::Albums;
                self.view = View::Grid;
                self.grid_scroll = 0.0;
                self.load_visible_art()
            }
            Message::Back => {
                if self.adding.is_some() {
                    self.adding = None;
                } else if self.tab == Tab::Playlists && self.list_view != ListView::All {
                    self.list_view = ListView::All;
                } else if self.view != View::Grid {
                    self.view = View::Grid;
                } else if self.artist_filter.is_some() {
                    self.artist_filter = None;
                } else {
                    self.query.clear();
                }
                self.load_visible_art()
            }
            Message::PlayAlbum(id, start) => self.player_post(
                "/api/player/play",
                Some(serde_json::json!({ "album": id, "start": start })),
            ),
            Message::AddAlbum(id) => self.player_post(
                "/api/player/queue",
                Some(serde_json::json!({ "album": id })),
            ),
            Message::Toggle => self.player_post("/api/player/toggle", None),
            Message::Next => self.player_post("/api/player/next", None),
            Message::Prev => self.player_post("/api/player/prev", None),
            Message::SeekFraction(f) => {
                let ms = (self.player.duration_ms as f32 * f) as u64;
                self.player_post(&format!("/api/player/seek?ms={ms}"), None)
            }
            Message::SeekMs(ms) => self.player_post(&format!("/api/player/seek?ms={ms}"), None),
            Message::Jump(i) => self.player_post(&format!("/api/player/jump?index={i}"), None),
            Message::GridScrolled(v) => {
                self.grid_scroll = v.absolute_offset().y;
                self.grid_height = v.bounds().height;
                self.load_visible_art()
            }
            Message::Resized(w, _h) => {
                self.width = w;
                self.load_visible_art()
            }
            // Development only: pretend the first track of the open album is playing,
            // with synced lyrics, so the now-playing card can be screenshotted
            // on a machine without a sound card.
            Message::DebugFake => {
                self.fake = true;
                let Some(open) = &self.open_album else {
                    return Task::none();
                };
                let Some(t) = open.tracks.first().cloned() else {
                    return Task::none();
                };
                let mut t = t;
                t.duration_ms = 254_000;
                self.player = Player {
                    state: PlayState::Playing,
                    track: Some(t.clone()),
                    position_ms: 97_000,
                    duration_ms: 254_000,
                    queue: open.tracks.iter().map(|x| x.id).collect(),
                    index: Some(0),
                    volume: 1.0,
                    device: "hw:1,0".into(),
                    format: Some("44100 Hz · 16 bit · 2 ch".into()),
                    bit_perfect: true,
                    error: None,
                    radio: None,
                    notice: None,
                };
                self.queue = Some(Queue {
                    index: Some(0),
                    tracks: open.tracks.clone(),
                });
                self.lyrics_for = Some(t.id);
                self.lyrics = Some(Lyrics {
                    track_id: t.id,
                    synced: true,
                    source: "lrclib".into(),
                    text: "[00:12.00]In the next world war\n[00:17.50]In a jackknifed juggernaut\n[00:22.00]I am born again\n[00:48.00]In the neon sign\n[00:52.00]Scrolling up and down\n[00:56.00]I am born again\n[01:30.00]In an interstellar burst\n[01:35.00]I am back to save the universe\n[02:10.00]In a deep deep sleep of the innocent\n[02:15.00]I am born again\n[02:40.00]In a fast German car\n[02:44.00]I'm amazed that I survived\n[02:48.00]An airbag saved my life".into(),
                });
                self.lyrics_line = Some(6);
                if self.fake_nolyrics {
                    return Task::done(Message::DebugNoLyrics);
                }
                Task::none()
            }
            Message::DebugNoLyrics => {
                self.fake_nolyrics = true;
                self.lyrics = None;
                self.lyrics_line = None;
                if let Some(t) = self.player.track.as_mut() {
                    t.has_lyrics = false;
                    self.lyrics_query = t.title.clone();
                }
                Task::none()
            }
            Message::Shot(shot) => {
                if let Some(path) = self.screenshot.take() {
                    let img = ::image::RgbaImage::from_raw(
                        shot.size.width,
                        shot.size.height,
                        shot.bytes.to_vec(),
                    );
                    match img {
                        Some(img) => match img.save(&path) {
                            Ok(()) => eprintln!("screenshot saved to {path}"),
                            Err(e) => eprintln!("screenshot: {e}"),
                        },
                        None => eprintln!("screenshot: bad buffer"),
                    }
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _status, _id| match event {
                iced::Event::Window(window::Event::Resized(size)) => {
                    Some(Message::Resized(size.width, size.height))
                }
                // The thumb button on a mouse, like in a browser.
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
                    iced::mouse::Button::Back,
                )) => Some(Message::Back),
                _ => None,
            }),
            keyboard::on_key_press(|key, modifiers| {
                if modifiers.command() || modifiers.alt() {
                    return None;
                }
                match key.as_ref() {
                    Key::Named(Named::Space) => Some(Message::Toggle),
                    Key::Named(Named::Escape) | Key::Named(Named::Backspace) => Some(Message::Back),
                    Key::Character("n") => Some(Message::Next),
                    Key::Character("p") => Some(Message::Prev),
                    Key::Character("r") => Some(Message::RadioOn(None)),
                    _ => None,
                }
            }),
        ])
    }

    // ---- view ------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        let body: Element<Message> = match (self.tab, &self.view) {
            (Tab::Albums, View::Album(id)) => self.album_view(*id),
            (Tab::Albums, View::Grid) => self.grid_view(),
            (Tab::Artists, _) => self.artists_view(),
            (Tab::Playlists, _) => match self.list_view {
                ListView::All => self.playlists_view(),
                ListView::One(id) => self.playlist_view(id),
            },
            (Tab::Queue, _) => self.queue_view(),
            (Tab::Lyrics, _) => self.lyrics_view(),
        };
        column![self.header(), body, self.now_playing()]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn header(&self) -> Element<'_, Message> {
        let wordmark = button(text("Katalog").size(26).font(BLACK).line_height(0.9))
            .style(style::plain)
            .padding(0)
            .on_press(Message::Back);
        let placeholder = match self
            .artist_filter
            .and_then(|id| self.artists.iter().find(|a| a.id == id))
        {
            Some(a) => format!("Albums by {}", a.name),
            None => "Search albums, artists, years".to_string(),
        };
        let search = text_input(&placeholder, &self.query)
            .id(text_input::Id::new("search"))
            .on_input(Message::Search)
            .size(20)
            .font(MEDIUM)
            .padding([4, 0])
            .style(style::search);
        let search =
            column![search, horizontal_rule(2).style(style::rule_px(CREAM, 2))].width(Length::Fill);

        let total = format!(
            "{} albums · {} tracks · {}",
            self.status.albums,
            self.status.tracks,
            fmt_long(self.status.duration_ms)
        );
        let top = row![
            wordmark,
            Space::with_width(GUTTER),
            search,
            Space::with_width(GUTTER),
            text(total).size(12).color(alpha(CREAM, 0.8)),
        ]
        .align_y(Alignment::End);

        let lyrics_note = match &self.lyrics {
            Some(l) if l.synced => "synced".to_string(),
            Some(_) => "plain".to_string(),
            None if self.player.track.is_some() => "none".to_string(),
            None => String::new(),
        };
        let tabs = row![
            self.tab_button(
                Tab::Albums,
                "Albums",
                self.filtered_albums().len().to_string()
            ),
            Space::with_width(CARD_GAP),
            self.tab_button(
                Tab::Artists,
                "Artists",
                self.filtered_artists().len().to_string()
            ),
            Space::with_width(CARD_GAP),
            self.tab_button(
                Tab::Playlists,
                "Playlists",
                self.playlists.len().to_string()
            ),
            Space::with_width(CARD_GAP),
            self.tab_button(Tab::Queue, "Queue", self.player.queue.len().to_string()),
            Space::with_width(CARD_GAP),
            self.tab_button(Tab::Lyrics, "Lyrics", lyrics_note),
            horizontal_space(),
        ];

        let mut head = column![top, Space::with_height(16), tabs].spacing(0);
        if let Some(e) = &self.error {
            head = head.push(Space::with_height(12)).push(
                row![
                    text(e.clone()).size(13).color(RED).font(BOLD),
                    horizontal_space(),
                    text("is katalogd running?")
                        .size(12)
                        .color(alpha(CREAM, 0.7)),
                ]
                .align_y(Alignment::End),
            );
        }
        container(column![
            head,
            Space::with_height(16),
            horizontal_rule(1).style(style::hairline(CREAM))
        ])
        .padding([16, GUTTER as u16])
        .padding(iced::Padding {
            top: 16.0,
            right: GUTTER,
            bottom: 0.0,
            left: GUTTER,
        })
        .width(Length::Fill)
        .into()
    }

    fn tab_button(&self, tab: Tab, label: &str, count: String) -> Element<'_, Message> {
        let active = self.tab == tab;
        let fg = if active { CREAM } else { alpha(CREAM, 0.6) };
        let label = text(label.to_string())
            .size(15)
            .font(if active { BLACK } else { MEDIUM })
            .color(fg)
            .line_height(1.1);
        let count = text(count).size(12).color(fg).line_height(1.0);
        let inner = column![
            row![label, Space::with_width(10), count].align_y(Alignment::End),
            Space::with_height(5),
            horizontal_rule(if active { 3 } else { 1 }).style(style::rule_px(
                if active {
                    CREAM
                } else {
                    iced::Color::TRANSPARENT
                },
                if active { 3 } else { 1 }
            )),
        ]
        .width(Length::Fixed(130.0));
        button(inner)
            .style(style::plain)
            .padding(0)
            .on_press(Message::Tab(tab))
            .into()
    }

    fn grid_view(&self) -> Element<'_, Message> {
        let albums = self.filtered_albums();
        let cols = self.columns();
        let w = self.card_width();
        if albums.is_empty() {
            return container(
                column![
                    text("Nothing found").size(44).font(BLACK).line_height(0.9),
                    Space::with_height(12),
                    horizontal_rule(2).style(style::rule_px(CREAM, 2)),
                    Space::with_height(24),
                    text(if self.albums.is_empty() {
                        "The library is empty or katalogd is still scanning."
                    } else {
                        "Try fewer words, or an artist, or a year."
                    })
                    .size(15)
                    .color(alpha(CREAM, 0.8)),
                ]
                .max_width(640.0),
            )
            .padding(iced::Padding::new(GUTTER).top(64.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }
        let mut grid = column![].spacing(ROW_GAP);
        for chunk in albums.chunks(cols) {
            let mut r = row![].spacing(CARD_GAP);
            for a in chunk {
                r = r.push(self.album_card(a, w));
            }
            // Keep the last row's cards the same size as the others.
            for _ in chunk.len()..cols {
                r = r.push(Space::with_width(Length::Fixed(w)));
            }
            grid = grid.push(r);
        }
        scrollable(container(grid).padding(iced::Padding {
            top: 32.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .id(scrollable::Id::new("grid"))
        .on_scroll(Message::GridScrolled)
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn cover<'a>(
        &self,
        art_id: Option<i64>,
        size: u32,
        side: f32,
        title: &str,
        artist: &str,
        seed: i64,
    ) -> Element<'a, Message> {
        if let Some(h) = art_id.and_then(|id| self.art.get(&(id, size))) {
            // The thumbnail is already square, so nothing can spill over the neighbours.
            return container(
                image(h.clone())
                    .width(Length::Fixed(side))
                    .height(Length::Fixed(side))
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .clip(true)
            .into();
        }
        // Typographic placeholder: the title set big on a colour block.
        let (bg, fg, acc) = style::placeholder_colors(seed);
        let pad = side * 0.08;
        let title_size = (side * 0.13).clamp(12.0, 52.0);
        let foot_size = (side * 0.05).clamp(8.0, 13.0);
        let mark = container(Space::new(Length::Fill, Length::Fixed(side * 0.16)))
            .style(style::filled(acc))
            .width(Length::Fill);
        let inner = column![
            text(title.to_string())
                .size(title_size)
                .font(BLACK)
                .line_height(0.9)
                .color(fg),
            vertical_space(),
            mark,
            vertical_space(),
            horizontal_rule(1).style(style::hairline(fg)),
            Space::with_height(pad * 0.4),
            row![
                text(artist.to_string()).size(foot_size).color(fg),
                horizontal_space(),
            ],
        ]
        .height(Length::Fill);
        container(inner)
            .padding(pad)
            .style(style::filled_text(bg, fg))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .clip(true)
            .into()
    }

    fn album_card<'a>(&self, a: &Album, w: f32) -> Element<'a, Message> {
        let cover = self.cover(a.art_id, THUMB, w, &a.title, &a.artist, a.id);
        let year = a.year.map(|y| y.to_string()).unwrap_or_default();
        let head = row![
            container(
                text(a.title.clone())
                    .size(18)
                    .font(BOLD)
                    .line_height(1.1)
                    .wrapping(text::Wrapping::None)
            )
            .width(Length::Fill)
            .clip(true),
            text(year)
                .size(13)
                .color(alpha(CREAM, 0.85))
                .wrapping(text::Wrapping::None),
        ]
        .align_y(Alignment::End)
        .spacing(8)
        .width(Length::Fixed(w));
        let card = column![
            cover,
            Space::with_height(16),
            container(head).clip(true).width(Length::Fixed(w)),
            Space::with_height(4),
            horizontal_rule(1).style(style::hairline(CREAM)),
            Space::with_height(6),
            text(a.artist.clone()).size(13).line_height(1.3),
        ]
        .width(Length::Fixed(w));
        mouse_area(card)
            .on_press(Message::OpenAlbum(a.id))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    fn album_view(&self, id: i64) -> Element<'_, Message> {
        let Some(open) = self.open_album.as_ref().filter(|a| a.album.id == id) else {
            return container(text("").size(15)).height(Length::Fill).into();
        };
        let a = &open.album;
        let side = ((self.width - 2.0 * GUTTER) * 0.36).clamp(240.0, 480.0);
        let cover = self.cover(a.art_id, 640, side, &a.title, &a.artist, a.id);

        let mut tracks = column![].spacing(0);
        for (i, t) in open.tracks.iter().enumerate() {
            let num = t.number.map(|n| n.to_string()).unwrap_or_default();
            tracks = tracks.push(self.track_row(t, num, None, Message::PlayAlbum(a.id, i), None));
        }

        let meta = format!(
            "{}{} tracks · {}",
            a.year.map(|y| format!("{y} · ")).unwrap_or_default(),
            a.track_count,
            fmt_long(a.duration_ms)
        );
        let title = column![
            text(a.title.clone())
                .size(if a.title.len() > 28 { 32 } else { 44 })
                .font(BLACK)
                .line_height(0.9),
            Space::with_height(10),
            horizontal_rule(2).style(style::rule_px(CREAM, 2)),
            Space::with_height(8),
            row![
                button(text(a.artist.clone()).size(15).line_height(1.2))
                    .style(style::plain)
                    .padding(0)
                    .on_press(Message::OpenArtist(a.artist_id)),
                horizontal_space(),
                text(meta).size(13).color(alpha(CREAM, 0.85)),
            ]
            .align_y(Alignment::End),
        ];
        let actions = row![
            chip("All albums", Message::Back),
            Space::with_width(24),
            chip("Play", Message::PlayAlbum(a.id, 0)),
            Space::with_width(8),
            chip("Add to queue", Message::AddAlbum(a.id)),
            horizontal_space(),
        ];
        let right = column![
            title,
            Space::with_height(24),
            actions,
            Space::with_height(24),
            tracks
        ]
        .width(Length::Fill);
        scrollable(
            container(row![cover, Space::with_width(48), right].align_y(Alignment::Start)).padding(
                iced::Padding {
                    top: 40.0,
                    right: GUTTER,
                    bottom: 48.0,
                    left: GUTTER,
                },
            ),
        )
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    /// One track line: a number or position on the left, the title, an
    /// optional note in the middle, the length and a "+" on the right that
    /// opens the playlist chooser underneath. `remove` adds a "×" for
    /// playlist pages.
    fn track_row<'a>(
        &'a self,
        t: &'a Track,
        left: String,
        sub: Option<String>,
        on_press: Message,
        remove: Option<Message>,
    ) -> Element<'a, Message> {
        let is_current = self.player.track.as_ref().map(|x| x.id) == Some(t.id);
        let op = if is_current { 1.0 } else { 0.8 };
        let mut r = row![
            container(text(left).size(13).color(alpha(CREAM, 0.7)))
                .width(Length::Fixed(36.0))
                .align_x(iced::alignment::Horizontal::Right),
            Space::with_width(16),
            text(t.title.clone())
                .size(15)
                .font(if is_current { BLACK } else { MEDIUM })
                .color(alpha(CREAM, op))
                .line_height(1.2),
        ]
        .align_y(Alignment::End);
        if let Some(sub) = sub {
            r = r
                .push(Space::with_width(16))
                .push(text(sub).size(13).color(alpha(CREAM, 0.7)).line_height(1.2));
        }
        r = r.push(horizontal_space());
        if t.has_lyrics {
            r = r
                .push(text("lyrics").size(12).color(alpha(CREAM, 0.6)))
                .push(Space::with_width(16));
        }
        r = r.push(
            container(text(fmt_ms(t.duration_ms)).size(13).color(alpha(CREAM, op)))
                .width(Length::Fixed(44.0))
                .align_x(iced::alignment::Horizontal::Right),
        );
        let adding = self.adding == Some(t.id);
        let small = |label: &str, m: Message, on: bool| {
            button(
                text(label.to_string())
                    .size(13)
                    .line_height(1.0)
                    .color(if on { CREAM } else { alpha(CREAM, 0.6) }),
            )
            .style(style::plain)
            .padding([2, 6])
            .on_press(m)
        };
        r = r
            .push(Space::with_width(12))
            .push(small("+", Message::AddTo(t.id), adding));
        if let Some(m) = remove {
            r = r.push(small("×", m, false));
        }
        let line = button(
            column![
                Space::with_height(12),
                r,
                Space::with_height(6),
                horizontal_rule(1).style(style::hairline(alpha(
                    CREAM,
                    if is_current { 1.0 } else { 0.5 }
                ))),
            ]
            .width(Length::Fill),
        )
        .style(style::plain)
        .padding(0)
        .on_press(on_press);
        if adding {
            column![line, self.add_chooser(t.id)].into()
        } else {
            line.into()
        }
    }

    /// Under a track: its playlists in bold, the others to click, and a box
    /// for a new one. Enter makes the playlist and puts the track in.
    fn add_chooser(&self, track_id: i64) -> Element<'_, Message> {
        let mut chips = row![].spacing(8).align_y(Alignment::Center);
        chips = chips.push(
            text("Add to")
                .size(13)
                .color(alpha(CREAM, 0.7))
                .line_height(1.0),
        );
        for p in &self.playlists {
            let has = self.adding_in.contains(&p.id);
            let b = button(
                text(p.name.clone())
                    .size(13)
                    .font(if has { BOLD } else { MEDIUM })
                    .line_height(1.0),
            )
            .style(style::chip)
            .padding([4, 10]);
            chips = chips.push(if has {
                b
            } else {
                b.on_press(Message::AddToPlaylist(p.id, track_id))
            });
        }
        let new = text_input("New playlist", &self.new_playlist)
            .id(text_input::Id::new("new-in-chooser"))
            .on_input(Message::NewPlaylistName)
            .on_submit(Message::CreateAndAdd(track_id))
            .size(13)
            .padding([4, 8])
            .width(Length::Fixed(160.0))
            .style(style::search);
        chips = chips.push(
            column![
                new,
                horizontal_rule(1).style(style::hairline(alpha(CREAM, 0.6)))
            ]
            .width(Length::Fixed(160.0)),
        );
        container(chips)
            .padding(iced::Padding {
                top: 10.0,
                right: 0.0,
                bottom: 10.0,
                left: 52.0,
            })
            .width(Length::Fill)
            .into()
    }

    fn playlists_view(&self) -> Element<'_, Message> {
        let new = text_input("New playlist", &self.new_playlist)
            .id(text_input::Id::new("new-playlist"))
            .on_input(Message::NewPlaylistName)
            .on_submit(Message::CreatePlaylist)
            .size(15)
            .padding([4, 0])
            .style(style::search);
        let head = row![
            column![new, horizontal_rule(1).style(style::hairline(CREAM))]
                .width(Length::Fixed(280.0)),
            Space::with_width(16),
            chip("Make it", Message::CreatePlaylist),
            horizontal_space(),
            chip(
                if self.player.radio.is_some() {
                    "Radio off"
                } else {
                    "Radio: everything"
                },
                if self.player.radio.is_some() {
                    Message::RadioOff
                } else {
                    Message::RadioOn(None)
                }
            ),
        ]
        .align_y(Alignment::Center);

        let mut list = column![head, Space::with_height(24)];
        if self.playlists.is_empty() {
            list = list.push(
                text("No playlists yet. Name one above, or press + on any song.")
                    .size(15)
                    .color(alpha(CREAM, 0.8)),
            );
        }
        for p in &self.playlists {
            let on_air = self.player.radio.as_ref().map(|r| r.playlist) == Some(Some(p.id));
            let r = row![
                text(p.name.clone()).size(20).line_height(1.1),
                Space::with_width(12),
                text(if on_air { "on air" } else { "" })
                    .size(12)
                    .color(alpha(CREAM, 0.7)),
                horizontal_space(),
                text(format!(
                    "{} {} · {}",
                    p.track_count,
                    if p.track_count == 1 {
                        "track"
                    } else {
                        "tracks"
                    },
                    fmt_long(p.duration_ms)
                ))
                .size(13)
                .color(alpha(CREAM, 0.8)),
            ]
            .align_y(Alignment::End);
            list = list.push(
                button(
                    column![
                        Space::with_height(14),
                        r,
                        Space::with_height(6),
                        horizontal_rule(1).style(style::hairline(CREAM)),
                    ]
                    .width(Length::Fill),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::OpenPlaylist(p.id)),
            );
        }
        scrollable(container(list).padding(iced::Padding {
            top: 24.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn playlist_view(&self, id: i64) -> Element<'_, Message> {
        let Some(open) = self.open_playlist.as_ref().filter(|p| p.playlist.id == id) else {
            return container(text("").size(15)).height(Length::Fill).into();
        };
        let p = &open.playlist;
        let on_air = self.player.radio.as_ref().map(|r| r.playlist) == Some(Some(p.id));
        let meta = format!(
            "{} {} · {}",
            p.track_count,
            if p.track_count == 1 {
                "track"
            } else {
                "tracks"
            },
            fmt_long(p.duration_ms)
        );
        let title = column![
            text(p.name.clone())
                .size(if p.name.len() > 28 { 32 } else { 44 })
                .font(BLACK)
                .line_height(0.9),
            Space::with_height(10),
            horizontal_rule(2).style(style::rule_px(CREAM, 2)),
            Space::with_height(8),
            row![
                text(if on_air { "on air" } else { "playlist" })
                    .size(15)
                    .line_height(1.2),
                horizontal_space(),
                text(meta).size(13).color(alpha(CREAM, 0.85)),
            ]
            .align_y(Alignment::End),
        ];
        let mut actions = row![
            chip("All playlists", Message::Back),
            Space::with_width(24),
            chip("Play", Message::PlayPlaylist(p.id, 0)),
            Space::with_width(8),
            chip("Add to queue", Message::AddPlaylist(p.id)),
            Space::with_width(8),
        ];
        actions = actions.push(if on_air {
            chip("Radio off", Message::RadioOff)
        } else {
            chip("Radio", Message::RadioOn(Some(p.id)))
        });
        actions = actions
            .push(horizontal_space())
            .push(chip("Delete", Message::DeletePlaylist(p.id)));

        let mut tracks = column![].spacing(0);
        if open.tracks.is_empty() {
            tracks = tracks.push(
                text("Nothing in it yet. Press + on a song in an album or the queue.")
                    .size(15)
                    .color(alpha(CREAM, 0.8)),
            );
        }
        for (i, t) in open.tracks.iter().enumerate() {
            tracks = tracks.push(self.track_row(
                t,
                (i + 1).to_string(),
                Some(format!("{} · {}", t.artist, t.album)),
                Message::PlayPlaylist(p.id, i),
                Some(Message::RemoveFromPlaylist(p.id, i)),
            ));
        }
        let body = column![
            title,
            Space::with_height(24),
            actions,
            Space::with_height(24),
            tracks
        ]
        .width(Length::Fill);
        scrollable(container(body).padding(iced::Padding {
            top: 40.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn artists_view(&self) -> Element<'_, Message> {
        let mut list = column![];
        for a in self.filtered_artists() {
            let r = row![
                text(a.name.clone()).size(20).line_height(1.1),
                horizontal_space(),
                text(format!(
                    "{} {} · {} tracks",
                    a.album_count,
                    if a.album_count == 1 {
                        "album"
                    } else {
                        "albums"
                    },
                    a.track_count
                ))
                .size(13)
                .color(alpha(CREAM, 0.8)),
            ]
            .align_y(Alignment::End);
            list = list.push(
                button(
                    column![
                        Space::with_height(14),
                        r,
                        Space::with_height(6),
                        horizontal_rule(1).style(style::hairline(CREAM)),
                    ]
                    .width(Length::Fill),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::OpenArtist(a.id)),
            );
        }
        scrollable(container(list).padding(iced::Padding {
            top: 24.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn queue_view(&self) -> Element<'_, Message> {
        let Some(q) = &self.queue else {
            return container(
                text("Nothing queued. Open an album and press Play, or press r for the radio.")
                    .size(15)
                    .color(alpha(CREAM, 0.8)),
            )
            .padding(iced::Padding::new(GUTTER).top(32.0))
            .height(Length::Fill)
            .into();
        };
        let mut list = column![];
        if let Some(r) = &self.player.radio {
            list = list.push(
                row![
                    text(format!("Radio: {}", r.name))
                        .size(15)
                        .color(alpha(CREAM, 0.85)),
                    Space::with_width(16),
                    chip("Off", Message::RadioOff),
                    horizontal_space(),
                ]
                .align_y(Alignment::Center),
            );
            list = list.push(Space::with_height(8));
        }
        for (i, t) in q.tracks.iter().enumerate() {
            list = list.push(self.track_row(
                t,
                (i + 1).to_string(),
                Some(format!("{} · {}", t.artist, t.album)),
                Message::Jump(i),
                None,
            ));
        }
        scrollable(container(list).padding(iced::Padding {
            top: 24.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn lyrics_view(&self) -> Element<'_, Message> {
        let Some(t) = &self.player.track else {
            return container(text("Nothing playing.").size(15).color(alpha(CREAM, 0.8)))
                .padding(iced::Padding::new(GUTTER).top(32.0))
                .height(Length::Fill)
                .into();
        };
        let Some(l) = &self.lyrics else {
            if t.has_lyrics {
                return container(text("Loading…").size(15).color(alpha(CREAM, 0.8)))
                    .padding(iced::Padding::new(GUTTER).top(32.0))
                    .height(Length::Fill)
                    .into();
            }
            return self.lyrics_search_view(t);
        };
        let lines = l.lines();
        let mut col = column![].spacing(0);
        for (i, ln) in lines.iter().enumerate() {
            let is_now = l.synced && self.lyrics_line == Some(i);
            let past = l.synced && self.lyrics_line.map(|c| i < c).unwrap_or(false);
            let size = if l.synced { 24 } else { 17 };
            let line_text = if ln.text.is_empty() {
                "·".to_string()
            } else {
                ln.text.clone()
            };
            let txt = text(line_text)
                .size(size)
                .font(if is_now { BLACK } else { MEDIUM })
                .color(alpha(
                    CREAM,
                    if is_now {
                        1.0
                    } else if past {
                        0.4
                    } else {
                        0.6
                    },
                ))
                .line_height(1.25);
            let el: Element<Message> = match ln.at_ms {
                Some(ms) => button(
                    container(txt)
                        .height(Length::Fixed(LYRIC_LINE_H))
                        .align_y(iced::alignment::Vertical::Center),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::SeekMs(ms))
                .into(),
                None => container(txt).padding([6, 0]).into(),
            };
            col = col.push(el);
        }
        let source = format!(
            "{} · {}",
            l.source,
            if l.synced {
                "synced, click a line to jump"
            } else {
                "plain"
            }
        );
        let head = row![
            text(format!("{} · {}", t.artist, t.title))
                .size(13)
                .color(alpha(CREAM, 0.8)),
            horizontal_space(),
            text(source).size(12).color(alpha(CREAM, 0.6)),
        ]
        .align_y(Alignment::End);
        scrollable(
            container(column![
                head,
                Space::with_height(24),
                col,
                Space::with_height(240)
            ])
            .padding(iced::Padding {
                top: 24.0,
                right: GUTTER,
                bottom: 48.0,
                left: GUTTER,
            }),
        )
        .id(scrollable::Id::new("lyrics"))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    /// No lyrics stored: a search box with the title in it, the artist as a
    /// switch, and the hits to pick from. Picking one saves it.
    fn lyrics_search_view(&self, t: &Track) -> Element<'_, Message> {
        let input = text_input("Search LRCLIB", &self.lyrics_query)
            .id(text_input::Id::new("lyrics-search"))
            .on_input(Message::LyricsQuery)
            .on_submit(Message::LyricsSearch(self.lyrics_with_artist))
            .size(20)
            .font(MEDIUM)
            .padding([4, 0])
            .style(style::search);
        let input = column![input, horizontal_rule(1).style(style::hairline(CREAM))]
            .width(Length::Fixed(420.0));
        let artist_chip = chip(
            &if self.lyrics_with_artist {
                format!("by {} ✓", t.artist)
            } else {
                format!("by {}", t.artist)
            },
            Message::LyricsSearch(!self.lyrics_with_artist),
        );
        let head = column![
            text(format!("No lyrics for {} · {} yet.", t.artist, t.title))
                .size(15)
                .color(alpha(CREAM, 0.8)),
            Space::with_height(20),
            row![
                input,
                Space::with_width(16),
                chip(
                    if self.lyrics_searching {
                        "Searching…"
                    } else {
                        "Search"
                    },
                    Message::LyricsSearch(self.lyrics_with_artist)
                ),
                Space::with_width(8),
                artist_chip,
                horizontal_space(),
            ]
            .align_y(Alignment::Center),
            Space::with_height(6),
            text("Edit the words if the file's title has a remaster or a feat. in it.")
                .size(12)
                .color(alpha(CREAM, 0.6)),
        ];

        let mut list = column![];
        if let Some(n) = &self.lyrics_note {
            list = list
                .push(Space::with_height(24))
                .push(text(n.clone()).size(15).color(alpha(CREAM, 0.8)));
        }
        if !self.lyrics_hits.is_empty() {
            list = list.push(Space::with_height(24));
        }
        for (i, h) in self.lyrics_hits.iter().enumerate() {
            let same_length = match (h.duration_ms, t.duration_ms) {
                (Some(d), have) if have > 0 => (d - have).abs() <= 5_000,
                _ => false,
            };
            let first_line = h
                .text
                .lines()
                .map(|l| {
                    // Skip timestamps and tags for the preview.
                    let l = l.trim();
                    match l.rfind(']') {
                        Some(p) if l.starts_with('[') => l[p + 1..].trim(),
                        _ => l,
                    }
                })
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            let r = row![
                container(text((i + 1).to_string()).size(13).color(alpha(CREAM, 0.7)))
                    .width(Length::Fixed(36.0))
                    .align_x(iced::alignment::Horizontal::Right),
                Space::with_width(16),
                column![
                    text(format!("{} · {}", h.artist, h.title))
                        .size(15)
                        .font(if same_length { BOLD } else { MEDIUM })
                        .line_height(1.2),
                    text(first_line)
                        .size(13)
                        .color(alpha(CREAM, 0.6))
                        .line_height(1.2)
                        .wrapping(text::Wrapping::None),
                ]
                .width(Length::Fill),
                Space::with_width(16),
                text(h.album.clone()).size(13).color(alpha(CREAM, 0.7)),
                Space::with_width(16),
                text(if h.synced { "synced" } else { "plain" })
                    .size(12)
                    .color(alpha(CREAM, 0.6)),
                Space::with_width(16),
                container(
                    text(h.duration_ms.map(fmt_ms).unwrap_or_default())
                        .size(13)
                        .color(alpha(CREAM, if same_length { 1.0 } else { 0.7 }))
                )
                .width(Length::Fixed(44.0))
                .align_x(iced::alignment::Horizontal::Right),
            ]
            .align_y(Alignment::Center);
            list = list.push(
                button(
                    column![
                        Space::with_height(10),
                        r,
                        Space::with_height(8),
                        horizontal_rule(1).style(style::hairline(alpha(CREAM, 0.5))),
                    ]
                    .width(Length::Fill),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::LyricsPick(i)),
            );
        }
        if !self.lyrics_hits.is_empty() {
            list = list.push(Space::with_height(8)).push(
                text("Click a hit to keep it. Bold means the length matches the file.")
                    .size(12)
                    .color(alpha(CREAM, 0.6)),
            );
        }
        scrollable(container(column![head, list]).padding(iced::Padding {
            top: 32.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    /// The black card at the bottom: cover, titles, transport, progress.
    fn now_playing(&self) -> Element<'_, Message> {
        let p = &self.player;
        let side = 72.0;
        let art: Element<Message> = match p
            .track
            .as_ref()
            .and_then(|t| t.art_id)
            .and_then(|id| self.art.get(&(id, 128)))
        {
            Some(h) => container(
                image(h.clone())
                    .width(Length::Fixed(side))
                    .height(Length::Fixed(side))
                    .content_fit(iced::ContentFit::Contain),
            )
            .style(style::outlined(CARD_LINE))
            .into(),
            None => container(Space::new(Length::Fixed(side), Length::Fixed(side)))
                .style(style::outlined(CARD_LINE_STRONG))
                .into(),
        };
        let art = match p.track.as_ref() {
            Some(t) => mouse_area(art)
                .on_press(Message::OpenAlbum(t.album_id))
                .interaction(iced::mouse::Interaction::Pointer)
                .into(),
            None => art,
        };

        let (title, sub): (String, String) = match &p.track {
            Some(t) => (t.title.clone(), format!("{} · {}", t.artist, t.album)),
            None => (
                "Nothing playing".into(),
                if p.queue.is_empty() {
                    "Open an album and press Play, or press space".into()
                } else {
                    format!("{} in queue", p.queue.len())
                },
            ),
        };
        let titles = column![
            text(title)
                .size(15)
                .font(BOLD)
                .color(CARD_TEXT)
                .line_height(1.2)
                .wrapping(text::Wrapping::None),
            Space::with_height(4),
            text(sub)
                .size(13)
                .color(CARD_SOFT)
                .line_height(1.2)
                .wrapping(text::Wrapping::None),
        ]
        .width(Length::FillPortion(3));

        let glyph = if p.state == PlayState::Playing {
            icons::Glyph::Pause
        } else {
            icons::Glyph::Play
        };
        let ctl = |g, size: f32, m: Message| {
            button(icons::icon(g, CARD_TEXT, size))
                .style(style::plain_in(CARD_TEXT))
                .padding(4)
                .on_press(m)
        };
        let transport = row![
            ctl(icons::Glyph::Prev, 18.0, Message::Prev),
            Space::with_width(12),
            ctl(glyph, 26.0, Message::Toggle),
            Space::with_width(12),
            ctl(icons::Glyph::Next, 18.0, Message::Next),
        ]
        .align_y(Alignment::Center);

        let fraction = if p.duration_ms > 0 {
            p.position_ms as f32 / p.duration_ms as f32
        } else {
            0.0
        };
        let times = text(format!(
            "{}  {}",
            fmt_ms(p.position_ms as i64),
            fmt_ms(p.duration_ms as i64)
        ))
        .size(12)
        .color(CARD_SOFT);
        let mut detail_parts: Vec<String> = Vec::new();
        if let Some(f) = &p.format {
            detail_parts.push(f.clone());
        }
        if p.bit_perfect {
            detail_parts.push("bit-perfect".into());
        }
        if p.volume < 0.999 {
            detail_parts.push(format!("volume {:.0}%", p.volume * 100.0));
        }
        if let (Some(i), false) = (p.index, p.queue.is_empty()) {
            detail_parts.push(format!("{}/{} in queue", i + 1, p.queue.len()));
        }
        if let Some(r) = &p.radio {
            detail_parts.push(format!("radio: {}", r.name));
        }
        if let Some(n) = &p.notice {
            detail_parts.push(n.clone());
        }
        if let Some(e) = &p.error {
            detail_parts.push(e.clone());
        }
        let detail = text(detail_parts.join(" · "))
            .size(12)
            .color(CARD_MUTE)
            .wrapping(text::Wrapping::None);

        let current_line = match (&self.lyrics, self.lyrics_line) {
            (Some(l), Some(i)) if l.synced => {
                l.lines().get(i).map(|x| x.text.clone()).unwrap_or_default()
            }
            _ => String::new(),
        };
        let progress_col = column![
            icons::progress(fraction, CARD_LINE_STRONG, WHITE, Message::SeekFraction),
            Space::with_height(2),
            row![detail, horizontal_space(), times,].align_y(Alignment::End),
        ]
        .width(Length::FillPortion(5));

        let lyric = container(
            text(current_line)
                .size(13)
                .color(CARD_SOFT)
                .line_height(1.2)
                .wrapping(text::Wrapping::None),
        )
        .width(Length::FillPortion(3))
        .align_x(iced::alignment::Horizontal::Right)
        .clip(true);

        let bar = row![
            art,
            Space::with_width(20),
            titles,
            Space::with_width(24),
            transport,
            Space::with_width(24),
            progress_col,
            Space::with_width(24),
            lyric,
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill);

        container(column![
            horizontal_rule(1).style(style::hairline(CARD_LINE_STRONG)),
            container(bar).padding(iced::Padding {
                top: 16.0,
                right: GUTTER,
                bottom: 16.0,
                left: GUTTER,
            }),
        ])
        .style(style::filled_text(CARD, CARD_TEXT))
        .width(Length::Fill)
        .into()
    }
}

const LYRIC_LINE_H: f32 = 38.0;

/// Percent-encoding for a query value.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// An outlined button in the running text size.
fn chip<'a>(label: &str, m: Message) -> iced::widget::Button<'a, Message> {
    button(text(label.to_string()).size(13))
        .style(style::chip)
        .padding([6, 12])
        .on_press(m)
}

/// "1977" is that year, "197" the whole decade, "70s" and "1970s" too,
/// "1975-1980" a range.
fn year_matches(year: i64, q: &str) -> bool {
    let y = year.to_string();
    if let Some((a, b)) = q.split_once('-') {
        if let (Ok(a), Ok(b)) = (a.trim().parse::<i64>(), b.trim().parse::<i64>()) {
            return (a.min(b)..=a.max(b)).contains(&year);
        }
    }
    if let Some(d) = q
        .strip_suffix('s')
        .filter(|d| d.chars().all(|c| c.is_ascii_digit()))
    {
        return match d.len() {
            2 => y.len() == 4 && y[2..3] == d[..1],
            4 => y.starts_with(&d[..3]),
            _ => false,
        };
    }
    !q.is_empty() && q.chars().all(|c| c.is_ascii_digit()) && y.starts_with(q)
}

#[cfg(test)]
mod tests {
    use super::year_matches;

    #[test]
    fn years_match_decades_and_ranges() {
        assert!(year_matches(1977, "1977"));
        assert!(year_matches(1977, "197"));
        assert!(!year_matches(1987, "197"));
        assert!(year_matches(1977, "70s"));
        assert!(year_matches(1977, "1970s"));
        assert!(!year_matches(1967, "70s"));
        assert!(year_matches(1977, "1975-1980"));
        assert!(!year_matches(1981, "1975-1980"));
        assert!(!year_matches(1977, "ok"));
    }
}
