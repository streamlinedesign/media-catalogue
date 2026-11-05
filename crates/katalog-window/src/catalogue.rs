//! The catalogue: albums, films and shows on one page, each section on its
//! own colour (navy, black, wine, as in the first catalogue's bar menu), with
//! an index on the left that stays put. And the settings, one page for music
//! and one for films and shows, in the same colours.

use crate::api::fmt_long;
use crate::video::length;
use crate::{
    chip, style, App, ListView, Message, Section, SettingsPage, CARD_GAP, CARD_TEXT_H, GUTTER,
    ROW_GAP,
};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, row, scrollable, text, Space,
};
use iced::{Alignment, Color, Element, Length};
use style::{alpha, BLACK, BOLD, CREAM, MEDIUM, NAVY, WINE};

/// The index on the left of the catalogue and the settings.
pub(crate) const INDEX_W: f32 = 180.0;
const SEC_TOP: f32 = 32.0;
const SEC_BOTTOM: f32 = 48.0;
const HEAD_H: f32 = 64.0;
const HEAD_GAP: f32 = 24.0;
const EMPTY_H: f32 = 40.0;
pub(crate) const FILMS: Color = Color::from_rgb(0.02, 0.02, 0.02);

pub(crate) fn section_color(s: Section) -> Color {
    match s {
        Section::Albums => NAVY,
        Section::Films => FILMS,
        Section::Shows => WINE,
    }
}

/// A square of a section's colour, for an index.
fn swatch<'a>(c: Color, lit: bool) -> Element<'a, Message> {
    container(Space::new(Length::Fixed(14.0), Length::Fixed(14.0)))
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(if lit { CREAM } else { c }.into()),
            border: iced::Border {
                color: CREAM,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn index_entry<'a>(
    c: Color,
    lit: bool,
    name: &str,
    count: String,
    m: Message,
) -> Element<'a, Message> {
    button(
        row![
            swatch(c, lit),
            Space::with_width(10),
            text(name.to_string())
                .size(15)
                .font(if lit { BLACK } else { MEDIUM })
                .color(if lit { CREAM } else { alpha(CREAM, 0.75) }),
            Space::with_width(8),
            text(count).size(12).color(alpha(CREAM, 0.7)),
        ]
        .align_y(Alignment::Center),
    )
    .style(style::plain)
    .padding([4, 0])
    .on_press(m)
    .into()
}

fn section_head<'a>(title: &str, meta: String) -> Element<'a, Message> {
    container(column![
        row![
            text(title.to_string())
                .size(40)
                .font(BLACK)
                .line_height(1.0),
            Space::with_width(16),
            text(meta).size(13).color(alpha(CREAM, 0.8)),
        ]
        .align_y(Alignment::End),
        Space::with_height(10),
        horizontal_rule(1).style(style::hairline(CREAM)),
    ])
    .height(Length::Fixed(HEAD_H))
    .into()
}

impl App {
    /// The width the catalogue's grids have.
    pub(crate) fn grid_width(&self) -> f32 {
        self.width - 2.0 * GUTTER - INDEX_W
    }

    fn cover_h(&self, s: Section) -> f32 {
        let w = self.card_width();
        match s {
            Section::Albums => w,
            _ => (w * 1.5).round(),
        }
    }

    fn section_len(&self, s: Section) -> usize {
        match s {
            Section::Albums => self.filtered_albums().len(),
            Section::Films => self.filtered_movies().len(),
            Section::Shows => self.filtered_shows().len(),
        }
    }

    /// Every row is the same height, so where a section starts is arithmetic.
    fn section_height(&self, s: Section) -> f32 {
        let rows = self.section_len(s).div_ceil(self.columns());
        let grid = if rows == 0 {
            EMPTY_H
        } else {
            rows as f32 * (self.cover_h(s) + CARD_TEXT_H) + (rows - 1) as f32 * ROW_GAP
        };
        SEC_TOP + HEAD_H + HEAD_GAP + grid + SEC_BOTTOM
    }

    pub(crate) fn section_top(&self, s: Section) -> f32 {
        match s {
            Section::Albums => 0.0,
            Section::Films => self.section_height(Section::Albums),
            Section::Shows => {
                self.section_height(Section::Albums) + self.section_height(Section::Films)
            }
        }
    }

    /// Where the album covers start, below the section's head.
    pub(crate) fn albums_grid_top(&self) -> f32 {
        SEC_TOP + HEAD_H + HEAD_GAP
    }

    /// The section the page is scrolled to.
    fn section_in_view(&self) -> Section {
        let y = self.grid_scroll + 80.0;
        let total = self.section_top(Section::Shows) + self.section_height(Section::Shows);
        // At the very bottom the last section may start lower than the page
        // can scroll: it is the one in view.
        let at_bottom = self.grid_scroll + self.grid_height >= total - 4.0;
        if at_bottom || y >= self.section_top(Section::Shows) {
            Section::Shows
        } else if y >= self.section_top(Section::Films) {
            Section::Films
        } else {
            Section::Albums
        }
    }

    fn rows<'a>(&self, cards: Vec<Element<'a, Message>>, row_h: f32) -> Element<'a, Message> {
        let cols = self.columns();
        let w = self.card_width();
        let mut grid = column![].spacing(ROW_GAP);
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            let mut r = row![].spacing(CARD_GAP);
            let mut n = 0;
            for c in cards.by_ref().take(cols) {
                r = r.push(c);
                n += 1;
            }
            // Keep the last row's cards the same size as the others.
            for _ in n..cols {
                r = r.push(Space::with_width(Length::Fixed(w)));
            }
            grid = grid.push(container(r).height(Length::Fixed(row_h)));
        }
        grid.into()
    }

    fn section<'a>(
        &self,
        s: Section,
        title: &str,
        meta: String,
        cards: Vec<Element<'a, Message>>,
        empty: &str,
    ) -> Element<'a, Message> {
        let body: Element<Message> = if cards.is_empty() {
            container(text(empty.to_string()).size(15).color(alpha(CREAM, 0.8)))
                .height(Length::Fixed(EMPTY_H))
                .into()
        } else {
            self.rows(cards, self.cover_h(s) + CARD_TEXT_H)
        };
        container(column![
            section_head(title, meta),
            Space::with_height(HEAD_GAP),
            body
        ])
        .padding(iced::Padding {
            top: SEC_TOP,
            right: GUTTER,
            bottom: SEC_BOTTOM,
            left: GUTTER,
        })
        .style(style::filled_text(section_color(s), CREAM))
        .width(Length::Fill)
        .into()
    }

    pub(crate) fn catalogue_view(&self) -> Element<'_, Message> {
        let w = self.card_width();
        let searching = !self.query.trim().is_empty();
        let albums = self.filtered_albums();
        let album_cards: Vec<_> = albums.iter().map(|a| self.album_card(a, w)).collect();
        let artists = {
            let mut a: Vec<i64> = albums.iter().map(|a| a.artist_id).collect();
            a.sort_unstable();
            a.dedup();
            a.len()
        };
        let films = self.filtered_movies();
        let film_cards: Vec<_> = films
            .iter()
            .map(|m| {
                let mut foot = vec![];
                if m.duration_ms > 0 {
                    foot.push(length(m.duration_ms));
                }
                if let Some(g) = m.genres.first() {
                    foot.push(g.clone());
                }
                self.poster_card(
                    m.poster_id,
                    &m.title,
                    m.year.map(|y| y.to_string()).unwrap_or_default(),
                    foot.join(" · "),
                    m.id,
                    w,
                    Message::OpenMovie(m.id),
                )
            })
            .collect();
        let film_ms: i64 = films.iter().map(|m| m.duration_ms).sum();
        let shows = self.filtered_shows();
        let show_cards: Vec<_> = shows
            .iter()
            .map(|s| {
                let seasons = if s.seasons == 1 {
                    "1 season".to_string()
                } else {
                    format!("{} seasons", s.seasons)
                };
                self.poster_card(
                    s.poster_id,
                    &s.title,
                    s.year.map(|y| y.to_string()).unwrap_or_default(),
                    format!("{seasons} · {} episodes", s.episode_count),
                    s.id,
                    w,
                    Message::OpenShow(s.id),
                )
            })
            .collect();
        let episodes: i64 = shows.iter().map(|s| s.episode_count).sum();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let none = "Nothing here matches.";
        let page = column![
            self.section(
                Section::Albums,
                "Albums",
                format!(
                    "{} · {}",
                    plural(albums.len(), "album", "albums"),
                    plural(artists, "artist", "artists")
                ),
                album_cards,
                if searching {
                    none
                } else if self.artist_filter.is_some() {
                    "No albums by this artist."
                } else {
                    "The library is empty, or katalogd is still scanning."
                },
            ),
            self.section(
                Section::Films,
                "Films",
                format!(
                    "{}{}",
                    plural(films.len(), "film", "films"),
                    if film_ms > 0 {
                        format!(" · {}", fmt_long(film_ms))
                    } else {
                        String::new()
                    }
                ),
                film_cards,
                if searching {
                    none
                } else {
                    "No films yet. katalogd reads them from Movies/ next to Music/."
                },
            ),
            self.section(
                Section::Shows,
                "Shows",
                format!(
                    "{} · {}",
                    plural(shows.len(), "show", "shows"),
                    plural(episodes as usize, "episode", "episodes")
                ),
                show_cards,
                if searching {
                    none
                } else {
                    "No shows yet. katalogd reads them from TV-Shows/, one folder a show."
                },
            ),
        ];
        let lit = self.section_in_view();
        let mut index = column![
            index_entry(
                NAVY,
                lit == Section::Albums,
                "Albums",
                albums.len().to_string(),
                Message::JumpTo(Section::Albums)
            ),
            index_entry(
                FILMS,
                lit == Section::Films,
                "Films",
                films.len().to_string(),
                Message::JumpTo(Section::Films)
            ),
            index_entry(
                WINE,
                lit == Section::Shows,
                "Shows",
                shows.len().to_string(),
                Message::JumpTo(Section::Shows)
            ),
        ]
        .spacing(10);
        if self.artist_filter.is_some() {
            index = index
                .push(Space::with_height(16))
                .push(chip("All artists", Message::Back));
        }
        row![
            container(index)
                .padding(iced::Padding {
                    top: 32.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: GUTTER,
                })
                .width(Length::Fixed(INDEX_W)),
            container(
                scrollable(page)
                    .id(scrollable::Id::new("grid"))
                    .on_scroll(Message::GridScrolled)
                    .style(style::scroll(CREAM))
                    .height(Length::Fill)
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true),
        ]
        .height(Length::Fill)
        .into()
    }

    // ---- settings ----------------------------------------------------------

    pub(crate) fn settings_view(&self) -> Element<'_, Message> {
        let index = column![
            index_entry(
                NAVY,
                self.settings == SettingsPage::Music,
                "Music",
                String::new(),
                Message::Settings(SettingsPage::Music)
            ),
            index_entry(
                FILMS,
                self.settings == SettingsPage::Screen,
                "Films",
                String::new(),
                Message::Settings(SettingsPage::Screen)
            ),
            index_entry(
                WINE,
                self.settings == SettingsPage::Screen,
                "Shows",
                String::new(),
                Message::Settings(SettingsPage::Screen)
            ),
        ]
        .spacing(10);
        let body = match self.settings {
            SettingsPage::Music => self.music_settings(),
            SettingsPage::Screen => self.screen_settings(),
        };
        row![
            container(index)
                .padding(iced::Padding {
                    top: 32.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: GUTTER,
                })
                .width(Length::Fixed(INDEX_W)),
            container(body).width(Length::Fill).height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }

    fn settings_note(&self) -> Element<'_, Message> {
        text(self.run_note.clone().unwrap_or_default())
            .size(13)
            .color(alpha(CREAM, 0.85))
            .into()
    }

    fn music_settings(&self) -> Element<'_, Message> {
        let lists = match self.list_view {
            ListView::All => self.playlists_view(),
            ListView::One(id) => self.playlist_view(id),
        };
        let library = column![
            line("Music folder", tilde(&self.status.library)),
            line(
                "Library",
                format!(
                    "{} albums · {} tracks · {}",
                    self.status.albums,
                    self.status.tracks,
                    fmt_long(self.status.duration_ms)
                )
            ),
            line(
                "Without a cover",
                format!("{} albums", self.status.albums_without_art)
            ),
            line(
                "Without lyrics",
                format!("{} tracks", self.status.tracks_without_lyrics)
            ),
            line(
                "CRT",
                if self.crt_on {
                    "on, c turns it off"
                } else {
                    "off, c turns it on"
                }
                .into()
            ),
            Space::with_height(12),
            row![
                chip("Scan now", Message::Run("/api/scan")),
                Space::with_width(8),
                chip("Fetch covers and lyrics", Message::Run("/api/fetch")),
            ],
            Space::with_height(8),
            row![chip(
                if self.crt_on { "CRT off" } else { "CRT on" },
                Message::Crt
            )],
            Space::with_height(8),
            self.settings_note(),
        ];
        row![
            column![settings_head("Playlists"), lists].width(Length::FillPortion(3)),
            column![settings_head("Queue"), self.queue_view()].width(Length::FillPortion(3)),
            column![
                settings_head("Library"),
                container(library).padding(iced::Padding {
                    top: 24.0,
                    right: GUTTER,
                    bottom: 0.0,
                    left: GUTTER,
                })
            ]
            .width(Length::FillPortion(2)),
        ]
        .height(Length::Fill)
        .into()
    }

    fn screen_settings(&self) -> Element<'_, Message> {
        let root = std::path::Path::new(&self.status.library)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let with_poster = self.movies.iter().filter(|m| m.poster_id.is_some()).count();
        let mut missing = column![].spacing(4);
        for m in self.movies.iter().filter(|m| m.poster_id.is_none()).take(8) {
            missing = missing.push(
                button(
                    text(format!(
                        "{}{}",
                        m.title,
                        m.year.map(|y| format!(" ({y})")).unwrap_or_default()
                    ))
                    .size(14),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::OpenMovie(m.id)),
            );
        }
        let films = column![
            text("Films").size(30).font(BLACK),
            Space::with_height(8),
            horizontal_rule(1).style(style::hairline(CREAM)),
            Space::with_height(16),
            line("Folder", tilde(&format!("{root}/Movies"))),
            line(
                "Subtitles",
                "English when a film has them, else none".into()
            ),
            line(
                "Extras",
                "folders below a film, and trailers, are left out".into()
            ),
            line(
                "Posters",
                format!("{with_poster} of {} films have one", self.movies.len())
            ),
            line("Tonight's film", "one a day, Another picks again".into()),
            Space::with_height(12),
            row![
                chip("Scan now", Message::Run("/api/scan")),
                Space::with_width(8),
                chip("Fetch posters", Message::Run("/api/fetch?kind=video")),
            ],
            Space::with_height(6),
            self.settings_note(),
            Space::with_height(24),
            text(if with_poster < self.movies.len() {
                "Without a poster"
            } else {
                ""
            })
            .size(18)
            .font(BOLD),
            Space::with_height(8),
            missing,
        ]
        .spacing(0);
        let mut channels = column![].spacing(0);
        for (i, c) in self.channels.iter().enumerate() {
            let what = match i {
                0 => "every film, shuffled, a new order each time round",
                1 => "every show in turn, one episode each, in order",
                _ => "films and episodes together",
            };
            channels = channels.push(
                button(
                    column![
                        Space::with_height(10),
                        row![
                            text(format!("{} {}", c.number, c.name)).size(18).font(BOLD),
                            horizontal_space(),
                            text(format!("{} items", c.items))
                                .size(13)
                                .color(alpha(CREAM, 0.8)),
                        ]
                        .align_y(Alignment::End),
                        Space::with_height(4),
                        text(what).size(13).color(alpha(CREAM, 0.85)),
                        Space::with_height(8),
                        horizontal_rule(1).style(style::hairline(alpha(CREAM, 0.4))),
                    ]
                    .width(Length::Fill),
                )
                .style(style::plain)
                .padding(0)
                .on_press(Message::TvChannel(i)),
            );
        }
        let shows = column![
            text("Shows").size(30).font(BLACK),
            Space::with_height(8),
            horizontal_rule(1).style(style::hairline(CREAM)),
            Space::with_height(16),
            line("Folder", tilde(&format!("{root}/TV-Shows"))),
            line(
                "The set",
                if self.tv.power {
                    "on, muted until clicked".into()
                } else {
                    "off until turned on".to_string()
                }
            ),
            line(
                "Sound",
                "fullscreen, or a film you play; the music pauses first".into()
            ),
            Space::with_height(12),
            row![chip(
                if self.tv.power {
                    "Turn the set off"
                } else {
                    "Turn the set on"
                },
                Message::TvPower
            )],
            Space::with_height(24),
            text("Channels").size(18).font(BOLD),
            channels,
        ];
        row![
            container(scrollable(container(films).padding(iced::Padding {
                top: 32.0,
                right: GUTTER,
                bottom: 32.0,
                left: GUTTER,
            })))
            .style(style::filled_text(FILMS, CREAM))
            .width(Length::FillPortion(1))
            .height(Length::Fill),
            container(scrollable(container(shows).padding(iced::Padding {
                top: 32.0,
                right: GUTTER,
                bottom: 32.0,
                left: GUTTER,
            })))
            .style(style::filled_text(WINE, CREAM))
            .width(Length::FillPortion(1))
            .height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }
}

fn settings_head<'a>(title: &str) -> Element<'a, Message> {
    container(column![
        text(title.to_string()).size(30).font(BLACK),
        Space::with_height(8),
        horizontal_rule(1).style(style::hairline(CREAM)),
    ])
    .padding(iced::Padding {
        top: 24.0,
        right: GUTTER,
        bottom: 0.0,
        left: GUTTER,
    })
    .into()
}

/// A path with the home folder as `~`.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && path.starts_with(&h) => format!("~{}", &path[h.len()..]),
        _ => path.to_string(),
    }
}

fn line<'a>(name: &str, value: String) -> Element<'a, Message> {
    column![
        row![
            text(name.to_string()).size(15),
            horizontal_space(),
            text(value).size(13).color(alpha(CREAM, 0.85)),
        ]
        .align_y(Alignment::End),
        Space::with_height(6),
        horizontal_rule(1).style(style::hairline(alpha(CREAM, 0.25))),
        Space::with_height(6),
    ]
    .into()
}
