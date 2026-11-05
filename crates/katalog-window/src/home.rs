//! Home: the dashboard. Green phosphor on a black screen, IBM Plex Mono, after
//! the Spectra-69 "Minimal" board: the cover and what is playing, a plain
//! spectrum with the transport under it, what comes next, the lyrics, and a
//! receiver whose stations are the playlists.

use crate::api::{fmt_ms, PlayState};
use crate::{icons, style, App, Message, Tab};
use iced::widget::canvas::{Canvas, Frame, Geometry, Program};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, image, mouse_area, row, text,
    vertical_space, Space,
};
use iced::{mouse, Alignment, Color, Element, Length, Point, Rectangle, Renderer, Theme};
use style::{
    MONO, MONO_LIGHT, MONO_MEDIUM, PHOSPHOR as G, PHOSPHOR_DIM as DIM, PHOSPHOR_LINE as LINE,
    SCREEN,
};

/// The three lists on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Next,
    Playlists,
    ForYou,
}

/// One position on the receiver's dial.
pub struct Station {
    pub playlist: Option<i64>,
    pub name: String,
    pub tracks: i64,
    pub mhz: f32,
}

const DIAL_LO: f32 = 87.5;
const DIAL_HI: f32 = 108.0;
const COVER_MAX: f32 = 320.0;
const BOTTOM_H: f32 = 196.0;

fn mono<'a>(s: impl ToString, size: f32, color: Color) -> iced::widget::Text<'a> {
    text(s.to_string())
        .size(size)
        .font(MONO)
        .color(color)
        .line_height(1.3)
}

fn label<'a>(s: &str) -> iced::widget::Text<'a> {
    mono(s.to_uppercase(), 11.0, DIM).font(MONO_MEDIUM)
}

fn rule<'a>() -> Element<'a, Message> {
    horizontal_rule(1).style(style::hairline(LINE)).into()
}

impl App {
    /// The playlists as stations, with "Everything" first, spread over the dial.
    pub(crate) fn stations(&self) -> Vec<Station> {
        let mut out = vec![Station {
            playlist: None,
            name: "Everything".into(),
            tracks: self.status.tracks,
            mhz: 0.0,
        }];
        out.extend(self.playlists.iter().map(|p| Station {
            playlist: Some(p.id),
            name: p.name.clone(),
            tracks: p.track_count,
            mhz: 0.0,
        }));
        let n = out.len() as f32;
        let span = DIAL_HI - DIAL_LO - 1.0;
        for (i, s) in out.iter_mut().enumerate() {
            let f = DIAL_LO + 0.5 + span * (i as f32 + 0.5) / n;
            s.mhz = (f * 10.0).round() / 10.0;
        }
        out
    }

    pub(crate) fn home_view(&self) -> Element<'_, Message> {
        let cover = (self.height - 64.0 - 52.0 - 48.0 - BOTTOM_H - 118.0).clamp(180.0, COVER_MAX);
        let top = row![
            container(self.home_now(cover)).width(Length::Fixed(cover)),
            container(self.home_deck()).width(Length::Fill),
            container(self.home_side()).width(Length::Fixed(340.0)),
        ]
        .spacing(40)
        .height(Length::Fill);
        let bottom = row![
            container(self.home_lyrics()).width(Length::Fill),
            self.home_receiver(),
        ]
        .spacing(40)
        .height(Length::Fixed(BOTTOM_H));
        container(
            column![self.home_header(), top, bottom]
                .spacing(24)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding([32, 40])
        .style(style::filled_text(SCREEN, G))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn home_header(&self) -> Element<'_, Message> {
        let tabs = [
            (Tab::Home, "Home"),
            (Tab::Albums, "Albums"),
            (Tab::Artists, "Artists"),
            (Tab::Playlists, "Playlists"),
            (Tab::Queue, "Queue"),
            (Tab::Lyrics, "Lyrics"),
        ];
        let mut nav = row![].spacing(24).align_y(Alignment::Center);
        for (t, name) in tabs {
            let lit = self.tab == t;
            nav = nav.push(
                button(
                    mono(name.to_uppercase(), 12.0, if lit { G } else { DIM }).font(if lit {
                        MONO_MEDIUM
                    } else {
                        MONO
                    }),
                )
                .style(style::screen_link(if lit { G } else { DIM }))
                .padding(0)
                .on_press(Message::Tab(t)),
            );
        }
        let source = match (&self.player.radio, &self.player.track) {
            (Some(r), _) => format!("radio · {}", r.name),
            (None, Some(t)) => t.album.clone(),
            (None, None) => "idle".into(),
        };
        let mut line = row![
            mono("KATALOG", 14.0, G).font(MONO_MEDIUM),
            Space::with_width(16),
            mono("home", 12.0, DIM),
            Space::with_width(48),
            nav,
            horizontal_space(),
            label("Source"),
            Space::with_width(10),
            mono(source, 12.0, G).wrapping(text::Wrapping::None),
        ]
        .align_y(Alignment::Center);
        if let Some(e) = &self.error {
            line = line.push(Space::with_width(24)).push(mono(
                format!("{e} · is katalogd running?"),
                12.0,
                G,
            ));
        }
        column![line, Space::with_height(16), rule()].into()
    }

    /// The cover, and the song under it.
    fn home_now(&self, side: f32) -> Element<'_, Message> {
        let p = &self.player;
        let art = p
            .track
            .as_ref()
            .and_then(|t| t.art_id)
            .and_then(|id| self.art.get(&(id, crate::HOME_ART)));
        let cover: Element<Message> = match art {
            Some(h) => container(
                image(h.clone())
                    .width(Length::Fixed(side))
                    .height(Length::Fixed(side))
                    .content_fit(iced::ContentFit::Cover),
            )
            .style(style::outlined(LINE))
            .into(),
            None => container(mono(
                if p.track.is_some() {
                    "NO ART"
                } else {
                    "NO SIGNAL"
                },
                14.0,
                DIM,
            ))
            .center(Length::Fixed(side))
            .style(style::outlined(LINE))
            .into(),
        };
        let cover: Element<Message> = match &p.track {
            Some(t) => mouse_area(cover)
                .on_press(Message::OpenAlbum(t.album_id))
                .interaction(mouse::Interaction::Pointer)
                .into(),
            None => cover,
        };
        let state = match p.state {
            PlayState::Playing => "Now playing",
            PlayState::Paused => "Paused",
            PlayState::Stopped => "Stopped",
        };
        let (title, artist, sub) = match &p.track {
            Some(t) => (
                t.title.clone(),
                t.artist.clone(),
                match t.year {
                    Some(y) => format!("{} · {y}", t.album),
                    None => t.album.clone(),
                },
            ),
            None => (
                "Nothing playing".into(),
                "Press space, or tune the receiver".into(),
                String::new(),
            ),
        };
        column![
            cover,
            Space::with_height(18),
            label(state),
            Space::with_height(6),
            mono(title, 24.0, G).font(MONO_MEDIUM).line_height(1.15),
            Space::with_height(6),
            mono(artist, 14.0, G),
            mono(sub, 13.0, DIM),
        ]
        .into()
    }

    /// Spectrum, time, progress and the keys.
    fn home_deck(&self) -> Element<'_, Message> {
        let p = &self.player;
        let mut fmt = Vec::new();
        if let Some(f) = &p.format {
            fmt.push(f.clone());
        }
        if p.bit_perfect {
            fmt.push("bit-perfect".into());
        }
        let spectrum = Canvas::new(Spectrum {
            bands: self.levels.clone(),
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let remain = p.duration_ms.saturating_sub(p.position_ms);
        let times = row![
            mono(fmt_ms(p.position_ms as i64), 40.0, G)
                .font(MONO_LIGHT)
                .line_height(1.0),
            horizontal_space(),
            mono(
                format!(
                    "−{} · {}",
                    fmt_ms(remain as i64),
                    fmt_ms(p.duration_ms as i64)
                ),
                14.0,
                DIM
            ),
        ]
        .align_y(Alignment::End);
        let fraction = if p.duration_ms > 0 {
            p.position_ms as f32 / p.duration_ms as f32
        } else {
            0.0
        };

        let key = |g, m: Message| {
            button(container(icons::icon(g, G, 18.0)).center(Length::Fixed(46.0)))
                .style(style::screen_key)
                .padding(0)
                .on_press(m)
        };
        let glyph = if p.state == PlayState::Playing {
            icons::Glyph::Pause
        } else {
            icons::Glyph::Play
        };
        let radio_on = p.radio.is_some();
        let radio_key = button(
            container(mono("RADIO", 12.0, if radio_on { SCREEN } else { G }).font(MONO_MEDIUM))
                .center_y(Length::Fixed(46.0))
                .padding([0, 14]),
        )
        .style(move |t, s| {
            let mut st = style::screen_key(t, s);
            if radio_on {
                st.background = Some(G.into());
                st.border.color = G;
            }
            st
        })
        .padding(0)
        .on_press(if radio_on {
            Message::RadioOff
        } else {
            Message::RadioOn(self.stations().get(self.tuned).and_then(|s| s.playlist))
        });

        let vol = (p.volume * 10.0).round() as usize;
        let mut segs = row![].spacing(3).align_y(Alignment::Center);
        for i in 0..10 {
            segs = segs.push(
                container(Space::new(Length::Fixed(6.0), Length::Fixed(16.0)))
                    .style(style::filled(if i < vol { G } else { LINE })),
            );
        }
        let vol_key = |s: &'static str, m: Message| {
            button(mono(s, 18.0, G))
                .style(style::screen_link(G))
                .padding([8, 12])
                .on_press(m)
        };
        let keys = row![
            key(icons::Glyph::Prev, Message::Prev),
            key(glyph, Message::Toggle),
            key(icons::Glyph::Next, Message::Next),
            Space::with_width(8),
            radio_key,
            horizontal_space(),
            vol_key("−", Message::Volume(-5)),
            segs,
            vol_key("+", Message::Volume(5)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        column![
            row![
                label("Spectrum"),
                horizontal_space(),
                mono(fmt.join(" · "), 11.0, DIM)
            ],
            Space::with_height(16),
            spectrum,
            rule(),
            Space::with_height(16),
            times,
            Space::with_height(10),
            icons::progress(fraction, LINE, G, Message::SeekFraction),
            Space::with_height(12),
            keys,
        ]
        .into()
    }

    /// Up next, the playlists, and the shelf.
    fn home_side(&self) -> Element<'_, Message> {
        let tab = |s: Side, name: &str| {
            let lit = self.side == s;
            button(
                column![
                    mono(name.to_uppercase(), 12.0, if lit { G } else { DIM }).font(MONO_MEDIUM),
                    Space::with_height(6),
                    horizontal_rule(2)
                        .style(style::rule_px(if lit { G } else { Color::TRANSPARENT }, 2)),
                ]
                .width(Length::Shrink),
            )
            .style(style::screen_link(DIM))
            .padding(0)
            .on_press(Message::Side(s))
        };
        let meta = match self.side {
            Side::Next => format!("{} in queue", self.player.queue.len()),
            Side::Playlists => format!("{}", self.playlists.len()),
            Side::ForYou => "picked for this album".into(),
        };
        let head = column![
            row![
                tab(Side::Next, "Up next"),
                Space::with_width(20),
                tab(Side::Playlists, "Playlists"),
                Space::with_width(20),
                tab(Side::ForYou, "For you"),
                horizontal_space(),
                mono(meta, 11.0, DIM),
            ]
            .align_y(Alignment::Start),
            rule(),
        ];

        let row_btn = |n: String, title: String, sub: String, right: String, m: Option<Message>| {
            let body = row![
                container(mono(n, 12.0, DIM))
                    .width(Length::Fixed(24.0))
                    .align_x(iced::alignment::Horizontal::Right),
                column![
                    mono(title, 14.0, G).wrapping(text::Wrapping::None),
                    mono(sub, 12.0, DIM).wrapping(text::Wrapping::None),
                ]
                .width(Length::Fill)
                .clip(true),
                mono(right, 12.0, DIM),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            let b = button(body)
                .style(style::screen_link(G))
                .padding([6, 0])
                .width(Length::Fill);
            match m {
                Some(m) => b.on_press(m),
                None => b,
            }
        };

        let mut list = column![].spacing(2);
        match self.side {
            Side::Next => {
                let start = self.player.index.map(|i| i + 1).unwrap_or(0);
                let tracks = self
                    .queue
                    .as_ref()
                    .map(|q| q.tracks.as_slice())
                    .unwrap_or(&[]);
                let mut any = false;
                for (k, t) in tracks.iter().enumerate().skip(start).take(7) {
                    any = true;
                    list = list.push(row_btn(
                        (k - start + 1).to_string(),
                        t.title.clone(),
                        t.artist.clone(),
                        fmt_ms(t.duration_ms),
                        Some(Message::Jump(k)),
                    ));
                }
                if !any {
                    list = list.push(mono(
                        if self.player.radio.is_some() {
                            "The radio picks the next song when this one starts."
                        } else {
                            "Nothing after this one."
                        },
                        13.0,
                        DIM,
                    ));
                }
            }
            Side::Playlists => {
                for (k, pl) in self.playlists.iter().enumerate().take(7) {
                    list = list.push(row_btn(
                        (k + 1).to_string(),
                        pl.name.clone(),
                        format!("{} tracks", pl.track_count),
                        fmt_ms(pl.duration_ms),
                        Some(Message::PlayPlaylist(pl.id, 0)),
                    ));
                }
                if self.playlists.is_empty() {
                    list = list.push(mono(
                        "No playlists yet. Add songs with + on any track.",
                        13.0,
                        DIM,
                    ));
                }
            }
            Side::ForYou => {
                for pk in &self.picks {
                    let a = &pk.album;
                    let year = a.year.map(|y| format!(" · {y}")).unwrap_or_default();
                    let queued = self.queued_albums.contains(&a.id);
                    let add = button(mono(if queued { "queued" } else { "+ queue" }, 12.0, G))
                        .style(style::screen_link(if queued { DIM } else { G }))
                        .padding([4, 0]);
                    let add = if queued {
                        add
                    } else {
                        add.on_press(Message::QueueAlbum(a.id))
                    };
                    list = list.push(
                        row![
                            button(
                                column![
                                    mono(a.title.clone(), 14.0, G).wrapping(text::Wrapping::None),
                                    mono(format!("{}{year}", a.artist), 12.0, DIM)
                                        .wrapping(text::Wrapping::None),
                                    mono(pk.why.clone(), 11.0, DIM),
                                ]
                                .width(Length::Fill)
                                .clip(true),
                            )
                            .style(style::screen_link(G))
                            .padding([6, 0])
                            .width(Length::Fill)
                            .on_press(Message::OpenAlbum(a.id)),
                            add,
                        ]
                        .spacing(12)
                        .align_y(Alignment::Center),
                    );
                }
                if self.picks.is_empty() {
                    list = list.push(mono("Play something to get picks.", 13.0, DIM));
                }
            }
        }
        column![head, Space::with_height(8), list].into()
    }

    /// Five lines around the one being sung.
    fn home_lyrics(&self) -> Element<'_, Message> {
        let tag = match &self.lyrics {
            Some(l) if l.synced => "synced",
            Some(_) => "plain",
            None if self.player.track.is_some() => "none",
            None => "",
        };
        let mut lines = column![].spacing(6);
        let line = |s: String, off: i32| {
            let (size, color) = match off.abs() {
                0 => (24.0, G),
                1 => (16.0, DIM),
                _ => (16.0, LINE),
            };
            mono(if s.is_empty() { " ".into() } else { s }, size, color)
                .font(if off == 0 { MONO_MEDIUM } else { MONO })
                .wrapping(text::Wrapping::None)
        };
        match &self.lyrics {
            Some(l) => {
                let all = l.lines();
                if l.synced {
                    let cur = self.lyrics_line.map(|i| i as i32).unwrap_or(-1);
                    for off in -2..=2 {
                        let k = cur + off;
                        let s = if cur < 0 && off == 0 {
                            "· · ·".to_string()
                        } else if k < 0 || k as usize >= all.len() {
                            String::new()
                        } else {
                            all[k as usize].text.clone()
                        };
                        lines = lines.push(line(s, off));
                    }
                } else {
                    for (k, ln) in all.iter().take(5).enumerate() {
                        lines = lines.push(line(ln.text.clone(), if k == 0 { 1 } else { 2 }));
                    }
                }
            }
            None => {
                lines = lines.push(line(String::new(), -1));
                lines = lines.push(line(
                    if self.player.track.is_some() {
                        "No lyrics for this song".into()
                    } else {
                        String::new()
                    },
                    1,
                ));
                if self.player.track.is_some() {
                    lines = lines.push(
                        button(mono("find them in the Lyrics tab", 13.0, DIM))
                            .style(style::screen_link(DIM))
                            .padding(0)
                            .on_press(Message::Tab(Tab::Lyrics)),
                    );
                }
            }
        }
        column![
            rule(),
            Space::with_height(16),
            row![label("Lyrics"), horizontal_space(), mono(tag, 11.0, DIM)],
            Space::with_height(12),
            container(lines).height(Length::Fill).clip(true),
        ]
        .into()
    }

    /// The receiver: the playlists on a dial, a power key, tuning and volume.
    fn home_receiver(&self) -> Element<'_, Message> {
        let stations = self.stations();
        let tuned = self.tuned.min(stations.len() - 1);
        let st = &stations[tuned];
        let on = self.player.radio.is_some();
        let power = button(
            row![
                container(Space::new(Length::Fixed(8.0), Length::Fixed(8.0)))
                    .style(style::filled(if on { G } else { LINE })),
                mono(
                    if on { "ON" } else { "OFF" },
                    11.0,
                    if on { G } else { DIM }
                )
                .font(MONO_MEDIUM),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .style(style::screen_link(G))
        .padding([4, 0])
        .on_press(Message::Power);

        // What the station is playing right now, tuned in or not.
        let on_now = match self
            .on_air
            .iter()
            .find(|a| a.playlist == st.playlist)
            .and_then(|a| a.now.as_ref())
        {
            Some(p) => format!("on air · {} · {}", p.artist, p.title),
            None if self.on_air.is_empty() => String::new(),
            None => "silent: nothing on this station yet".into(),
        };
        let marks: Vec<f32> = stations.iter().map(|s| s.mhz).collect();
        let dial = Canvas::new(Dial {
            marks,
            needle: st.mhz,
        })
        .width(Length::Fill)
        .height(Length::Fixed(22.0));
        let mut scale = row![];
        for (i, n) in ["88", "92", "96", "100", "104", "108"].iter().enumerate() {
            if i > 0 {
                scale = scale.push(horizontal_space());
            }
            scale = scale.push(mono(*n, 10.0, DIM));
        }

        let readout = column![
            row![label("FM receiver"), horizontal_space(), power].align_y(Alignment::Center),
            Space::with_height(6),
            mono(st.name.clone(), 12.0, DIM).wrapping(text::Wrapping::None),
            row![
                mono(format!("{:.1}", st.mhz), 30.0, G)
                    .font(MONO_LIGHT)
                    .line_height(1.1),
                Space::with_width(8),
                mono("MHz", 12.0, DIM),
            ]
            .align_y(Alignment::End),
            mono(on_now, 12.0, G).wrapping(text::Wrapping::None),
            mono(
                format!(
                    "{} tracks · station {} of {}",
                    st.tracks,
                    tuned + 1,
                    stations.len()
                ),
                11.0,
                DIM
            ),
            vertical_space(),
            dial,
            scale,
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        let n = stations.len().max(2) as f32 - 1.0;
        let knob = |name: &'static str, turn: f32, down: Message, up: Message| {
            let arrow = |s: &'static str, m: Message| {
                button(mono(s, 16.0, G))
                    .style(style::screen_link(G))
                    .padding([10, 8])
                    .on_press(m)
            };
            column![
                row![
                    arrow("‹", down),
                    Canvas::new(Knob { turn })
                        .width(Length::Fixed(48.0))
                        .height(Length::Fixed(48.0)),
                    arrow("›", up),
                ]
                .align_y(Alignment::Center),
                container(mono(name, 10.0, DIM).font(MONO_MEDIUM))
                    .width(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center),
            ]
            .width(Length::Fixed(128.0))
        };
        let knobs = column![
            knob(
                "TUNING",
                tuned as f32 / n,
                Message::Tune(-1),
                Message::Tune(1)
            ),
            vertical_space(),
            knob(
                "VOLUME",
                self.player.volume,
                Message::Volume(-5),
                Message::Volume(5)
            ),
        ]
        .height(Length::Fill);

        container(row![readout, knobs].spacing(16))
            .padding(16)
            .style(style::outlined(LINE))
            .width(Length::Fixed(560.0))
            .height(Length::Fill)
            .into()
    }
}

/// Bars from the levels, each a whole number of pixels.
struct Spectrum {
    bands: Vec<f32>,
}

impl<M> Program<M> for Spectrum {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let n = if self.bands.is_empty() {
            32
        } else {
            self.bands.len()
        };
        let gap = 3.0;
        let w = bounds.width.floor();
        let h = bounds.height.floor();
        let bar = ((w - gap * (n as f32 - 1.0)) / n as f32).floor().max(1.0);
        let used = bar * n as f32 + gap * (n as f32 - 1.0);
        let x0 = ((w - used) / 2.0).floor();
        for i in 0..n {
            let v = self.bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let bh = (v * h).round().max(1.0);
            let x = x0 + i as f32 * (bar + gap);
            let color = if bh <= 1.0 { LINE } else { G };
            frame.fill_rectangle(Point::new(x, h - bh), iced::Size::new(bar, bh), color);
        }
        vec![frame.into_geometry()]
    }
}

/// The dial: a tick per MHz, a mark per station, the needle.
struct Dial {
    marks: Vec<f32>,
    needle: f32,
}

impl<M> Program<M> for Dial {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width.floor() - 1.0;
        let h = bounds.height.floor();
        let x = |mhz: f32| ((mhz - 88.0) / 20.0 * w).round().clamp(0.0, w);
        let mut f = 88.0;
        while f <= 108.0 {
            let tall = (f as i32) % 4 == 0;
            frame.fill_rectangle(
                Point::new(x(f), 0.0),
                iced::Size::new(1.0, if tall { 10.0 } else { 6.0 }),
                LINE,
            );
            f += 1.0;
        }
        frame.fill_rectangle(Point::new(0.0, 10.0), iced::Size::new(w + 1.0, 1.0), LINE);
        for m in &self.marks {
            frame.fill_rectangle(Point::new(x(*m), 4.0), iced::Size::new(1.0, 10.0), DIM);
        }
        frame.fill_rectangle(
            Point::new(x(self.needle) - 1.0, 0.0),
            iced::Size::new(2.0, h),
            G,
        );
        vec![frame.into_geometry()]
    }
}

/// A knob: a ring and a pointer, `turn` from 0.0 (seven o'clock) to 1.0 (five o'clock).
struct Knob {
    turn: f32,
}

impl<M> Program<M> for Knob {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let c = Point::new((bounds.width / 2.0).round(), (bounds.height / 2.0).round());
        let r = (bounds.width.min(bounds.height) / 2.0 - 2.0).floor();
        // Built from whole-pixel squares: the software renderer drops thin
        // curves and shapes on fractional coordinates.
        let dot = |frame: &mut Frame, x: f32, y: f32, s: f32| {
            frame.fill_rectangle(Point::new(x.round(), y.round()), iced::Size::new(s, s), G);
        };
        let steps = (r * 6.3).ceil() as usize;
        for i in 0..steps {
            let t = i as f32 / steps as f32 * std::f32::consts::TAU;
            dot(&mut frame, c.x + r * t.cos(), c.y + r * t.sin(), 1.0);
        }
        // Screen angles run clockwise from three o'clock; seven o'clock is 120°.
        let a = (120.0 + 270.0 * self.turn.clamp(0.0, 1.0)).to_radians();
        let len = r - 6.0;
        let mut d = 0.0;
        while d <= len {
            dot(
                &mut frame,
                c.x + d * a.cos() - 1.0,
                c.y + d * a.sin() - 1.0,
                2.0,
            );
            d += 1.0;
        }
        vec![frame.into_geometry()]
    }
}
