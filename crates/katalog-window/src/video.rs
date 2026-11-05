//! The Movies and TV tabs: posters on a grid, and a page for each film and
//! show. Films sit on black, TV on wine, as in the first catalogue's bar menu.

use crate::api::{fmt_long, fmt_ms, Episode, Movie, Show};
use crate::{chip, style, App, Message, Tab, CARD_GAP, GUTTER, ROW_GAP};
use iced::widget::{
    column, container, horizontal_rule, horizontal_space, image, mouse_area, row, scrollable, text,
    Space,
};
use iced::{Alignment, Color, Element, Length};
use style::{alpha, BLACK, BOLD, CREAM};

/// Poster thumbnails: fetched uncropped, so they keep their 2:3.
pub const POSTER: u32 = 480;

/// The page colour behind each tab.
pub fn page_color(tab: Tab) -> Option<Color> {
    match tab {
        Tab::Movies => Some(Color::from_rgb8(0x05, 0x05, 0x05)),
        Tab::Tv => Some(style::WINE),
        _ => None,
    }
}

fn matches(q: &str, title: &str, year: Option<i64>, genres: &[String]) -> bool {
    q.is_empty()
        || title.to_lowercase().contains(q)
        || year.is_some_and(|y| crate::year_matches(y, q))
        || genres.iter().any(|g| g.to_lowercase().contains(q))
}

/// "1 h 49 min"; short test clips in seconds.
fn length(ms: i64) -> String {
    if ms >= 60_000 {
        fmt_long(ms)
    } else {
        fmt_ms(ms)
    }
}

/// "Horror, Science Fiction" → "Horror · Science Fiction".
fn genres(g: &[String]) -> String {
    g.join(" · ")
}

/// "en" first, as the player will pick it.
fn subs_line(subs: &[String]) -> String {
    if subs.is_empty() {
        return "no subtitles".into();
    }
    let names: Vec<String> = subs
        .iter()
        .map(|l| match l.as_str() {
            "en" => "English".to_string(),
            "hr" => "Croatian".to_string(),
            "sr" => "Serbian".to_string(),
            "bs" => "Bosnian".to_string(),
            "sl" => "Slovenian".to_string(),
            "de" => "German".to_string(),
            "fr" => "French".to_string(),
            "es" => "Spanish".to_string(),
            "it" => "Italian".to_string(),
            "?" => "unnamed".to_string(),
            other => other.to_string(),
        })
        .collect();
    format!("subtitles: {}", names.join(", "))
}

impl App {
    pub(crate) fn filtered_movies(&self) -> Vec<&Movie> {
        let q = self.query.trim().to_lowercase();
        self.movies
            .iter()
            .filter(|m| matches(&q, &m.title, m.year, &m.genres))
            .collect()
    }

    pub(crate) fn filtered_shows(&self) -> Vec<&Show> {
        let q = self.query.trim().to_lowercase();
        self.shows
            .iter()
            .filter(|s| matches(&q, &s.title, s.year, &s.genres))
            .collect()
    }

    /// Posters for everything on the tab. A few hundred small JPEGs at most.
    pub(crate) fn load_posters(&mut self) -> iced::Task<Message> {
        let ids: Vec<i64> = match self.tab {
            Tab::Movies => self.movies.iter().filter_map(|m| m.poster_id).collect(),
            Tab::Tv => self.shows.iter().filter_map(|s| s.poster_id).collect(),
            _ => return iced::Task::none(),
        };
        iced::Task::batch(ids.into_iter().map(|id| self.want_art(id, POSTER)))
    }

    fn poster<'a>(&self, id: Option<i64>, title: &str, seed: i64, w: f32) -> Element<'a, Message> {
        let h = (w * 1.5).round();
        if let Some(handle) = id.and_then(|id| self.art.get(&(id, POSTER))) {
            return container(
                image(handle.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .content_fit(iced::ContentFit::Cover),
            )
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .clip(true)
            .into();
        }
        // No poster: the title set big, like a title card.
        let (bg, fg, acc) = style::placeholder_colors(seed);
        let pad = w * 0.08;
        container(
            column![
                text(title.to_string())
                    .size((w * 0.14).clamp(12.0, 56.0))
                    .font(BLACK)
                    .line_height(0.9)
                    .color(fg),
                iced::widget::vertical_space(),
                container(Space::new(Length::Fill, Length::Fixed(w * 0.12)))
                    .style(style::filled(acc))
                    .width(Length::Fill),
            ]
            .height(Length::Fill),
        )
        .padding(pad)
        .style(style::filled_text(bg, fg))
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .clip(true)
        .into()
    }

    fn poster_card<'a>(
        &self,
        poster_id: Option<i64>,
        title: &str,
        right: String,
        foot: String,
        seed: i64,
        w: f32,
        open: Message,
    ) -> Element<'a, Message> {
        let head = row![
            container(
                text(title.to_string())
                    .size(18)
                    .font(BOLD)
                    .line_height(1.1)
                    .wrapping(text::Wrapping::None)
            )
            .width(Length::Fill)
            .clip(true),
            text(right)
                .size(13)
                .color(alpha(CREAM, 0.85))
                .wrapping(text::Wrapping::None),
        ]
        .align_y(Alignment::End)
        .spacing(8)
        .width(Length::Fixed(w));
        let card = column![
            self.poster(poster_id, title, seed, w),
            Space::with_height(16),
            container(head).clip(true).width(Length::Fixed(w)),
            Space::with_height(4),
            horizontal_rule(1).style(style::hairline(CREAM)),
            Space::with_height(6),
            container(
                text(foot)
                    .size(13)
                    .line_height(1.3)
                    .wrapping(text::Wrapping::None)
            )
            .width(Length::Fixed(w))
            .clip(true),
        ]
        .width(Length::Fixed(w));
        mouse_area(card)
            .on_press(open)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    fn poster_grid<'a>(
        &self,
        cards: Vec<Element<'a, Message>>,
        empty: &str,
    ) -> Element<'a, Message> {
        if cards.is_empty() {
            return container(
                column![
                    text("Nothing found").size(44).font(BLACK).line_height(0.9),
                    Space::with_height(12),
                    horizontal_rule(2).style(style::rule_px(CREAM, 2)),
                    Space::with_height(24),
                    text(empty.to_string()).size(15).color(alpha(CREAM, 0.8)),
                ]
                .max_width(640.0),
            )
            .padding(iced::Padding::new(GUTTER).top(64.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }
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
            for _ in n..cols {
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
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    pub(crate) fn movies_view(&self) -> Element<'_, Message> {
        if let Some(m) = self
            .open_movie
            .and_then(|id| self.movies.iter().find(|m| m.id == id))
        {
            return self.movie_view(m);
        }
        let w = self.card_width();
        let cards = self
            .filtered_movies()
            .into_iter()
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
        self.poster_grid(
            cards,
            if self.movies.is_empty() {
                "No films yet. katalogd reads them from Movies/ next to Music/."
            } else {
                "Try fewer words, a year, or a genre."
            },
        )
    }

    pub(crate) fn tv_view(&self) -> Element<'_, Message> {
        if let Some(open) = self
            .open_show
            .as_ref()
            .filter(|s| Some(s.show.id) == self.show_open)
        {
            return self.show_view(&open.show, &open.episodes);
        }
        let w = self.card_width();
        let cards = self
            .filtered_shows()
            .into_iter()
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
        self.poster_grid(
            cards,
            if self.shows.is_empty() {
                "No shows yet. katalogd reads them from TV-Shows/ next to Music/, one folder a show."
            } else {
                "Try fewer words, a year, or a genre."
            },
        )
    }

    /// The left column of a film or show page: the poster, as big as fits.
    fn page_side(&self) -> f32 {
        ((self.width - 2.0 * GUTTER) * 0.28).clamp(220.0, 400.0)
    }

    fn title_block<'a>(&self, title: &str, kind: &str, meta: String) -> Element<'a, Message> {
        column![
            text(title.to_string())
                .size(if title.len() > 28 { 32 } else { 44 })
                .font(BLACK)
                .line_height(0.9),
            Space::with_height(10),
            horizontal_rule(2).style(style::rule_px(CREAM, 2)),
            Space::with_height(8),
            row![
                text(kind.to_string()).size(15).line_height(1.2),
                horizontal_space(),
                text(meta).size(13).color(alpha(CREAM, 0.85)),
            ]
            .align_y(Alignment::End),
        ]
        .into()
    }

    fn movie_view<'a>(&'a self, m: &'a Movie) -> Element<'a, Message> {
        let side = self.page_side();
        let mut meta = vec![];
        if let Some(d) = &m.released {
            meta.push(d.clone());
        } else if let Some(y) = m.year {
            meta.push(y.to_string());
        }
        if m.duration_ms > 0 {
            meta.push(length(m.duration_ms));
        }
        let mut info = column![
            self.title_block(&m.title, "film", meta.join(" · ")),
            Space::with_height(16),
            row![chip("All films", Message::Back)],
            Space::with_height(28),
        ];
        if let Some(o) = &m.overview {
            info = info
                .push(text(o.clone()).size(16).line_height(1.4))
                .push(Space::with_height(20));
        }
        if !m.genres.is_empty() {
            info = info.push(text(genres(&m.genres)).size(13).color(alpha(CREAM, 0.85)));
        }
        info = info
            .push(Space::with_height(6))
            .push(text(subs_line(&m.subs)).size(13).color(alpha(CREAM, 0.85)));
        let body = row![
            self.poster(m.poster_id, &m.title, m.id, side),
            Space::with_width(GUTTER),
            info.width(Length::Fill).max_width(720.0),
        ];
        scrollable(container(body).padding(iced::Padding {
            top: 32.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }

    fn show_view<'a>(&'a self, s: &'a Show, episodes: &'a [Episode]) -> Element<'a, Message> {
        let side = self.page_side();
        let mut meta = vec![];
        if let Some(d) = &s.first_aired {
            meta.push(format!("from {d}"));
        } else if let Some(y) = s.year {
            meta.push(y.to_string());
        }
        meta.push(format!("{} episodes", s.episode_count));
        let mut info = column![
            self.title_block(&s.title, "series", meta.join(" · ")),
            Space::with_height(16),
            row![chip("All shows", Message::Back)],
            Space::with_height(28),
        ];
        if let Some(o) = &s.overview {
            info = info
                .push(text(o.clone()).size(16).line_height(1.4))
                .push(Space::with_height(20));
        }
        if !s.genres.is_empty() {
            info = info
                .push(text(genres(&s.genres)).size(13).color(alpha(CREAM, 0.85)))
                .push(Space::with_height(20));
        }
        let mut season = None;
        for e in episodes {
            if season != Some(e.season) {
                season = Some(e.season);
                info = info
                    .push(Space::with_height(20))
                    .push(text(format!("Season {}", e.season)).size(22).font(BLACK))
                    .push(Space::with_height(6))
                    .push(horizontal_rule(1).style(style::hairline(CREAM)));
            }
            let title = e
                .title
                .clone()
                .unwrap_or_else(|| format!("Episode {}", e.episode));
            let subs = if e.subs.is_empty() {
                String::new()
            } else {
                e.subs.join(" ")
            };
            info = info.push(
                container(
                    row![
                        text(format!("{:>2}", e.episode))
                            .size(13)
                            .color(alpha(CREAM, 0.7))
                            .width(Length::Fixed(36.0)),
                        container(text(title).size(15).wrapping(text::Wrapping::None))
                            .width(Length::Fill)
                            .clip(true),
                        text(subs).size(12).color(alpha(CREAM, 0.7)),
                        Space::with_width(16),
                        text(if e.duration_ms > 0 {
                            fmt_ms(e.duration_ms)
                        } else {
                            String::new()
                        })
                        .size(13)
                        .color(alpha(CREAM, 0.85)),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([8, 0]),
            );
        }
        let body = row![
            self.poster(s.poster_id, &s.title, s.id, side),
            Space::with_width(GUTTER),
            info.width(Length::Fill).max_width(720.0),
        ];
        scrollable(container(body).padding(iced::Padding {
            top: 32.0,
            right: GUTTER,
            bottom: 48.0,
            left: GUTTER,
        }))
        .style(style::scroll(CREAM))
        .height(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_named_first() {
        assert_eq!(
            subs_line(&["en".into(), "hr".into()]),
            "subtitles: English, Croatian"
        );
        assert_eq!(subs_line(&[]), "no subtitles");
    }
}
