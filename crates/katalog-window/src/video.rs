//! Films and shows: posters, tonight's film, and a page for each film and
//! show. Films sit on black, shows on wine, as in the first catalogue's bar
//! menu.

use crate::api::{fmt_long, fmt_ms, Episode, Movie, Show};
use crate::{chip, style, App, Message, Tab, GUTTER};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, image, mouse_area, row,
    scrollable, text, Space,
};
use iced::{Alignment, Element, Length};
use style::{alpha, BLACK, BOLD, CREAM};

/// Poster thumbnails: fetched uncropped, so they keep their 2:3.
pub const POSTER: u32 = 480;

fn matches(q: &str, title: &str, year: Option<i64>, genres: &[String]) -> bool {
    q.is_empty()
        || title.to_lowercase().contains(q)
        || year.is_some_and(|y| crate::year_matches(y, q))
        || genres.iter().any(|g| g.to_lowercase().contains(q))
}

/// "1 h 49 min"; short test clips in seconds.
pub(crate) fn length(ms: i64) -> String {
    if ms >= 60_000 {
        fmt_long(ms)
    } else {
        fmt_ms(ms)
    }
}

/// "Horror, Science Fiction" → "Horror · Science Fiction".
pub(crate) fn genres(g: &[String]) -> String {
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

    /// Every poster, for the pages that show them. A few hundred small
    /// JPEGs at most.
    pub(crate) fn load_posters(&mut self) -> iced::Task<Message> {
        if self.tab == Tab::Music || self.tab == Tab::Lyrics {
            return iced::Task::none();
        }
        let ids: Vec<i64> = self
            .movies
            .iter()
            .filter_map(|m| m.poster_id)
            .chain(self.shows.iter().filter_map(|s| s.poster_id))
            .collect();
        iced::Task::batch(ids.into_iter().map(|id| self.want_art(id, POSTER)))
    }

    pub(crate) fn poster<'a>(
        &self,
        id: Option<i64>,
        title: &str,
        seed: i64,
        w: f32,
    ) -> Element<'a, Message> {
        let h = (w * 1.5).round();
        if let Some(handle) = id.and_then(|id| self.art.get(&(id, POSTER))) {
            return container(
                image(handle.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    // Contain, not Cover: a picture that overflows its box is
                    // drawn on a layer of its own, which escapes the clip of a
                    // scrolled page. Posters are 2:3 anyway.
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(w))
            .height(Length::Fixed(h))
            .center_x(Length::Fixed(w))
            .style(style::filled(iced::Color::BLACK))
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn poster_card<'a>(
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

    /// Tonight's film: one a day, the same all day, chosen from those with a
    /// poster when there are any. "Another" moves on to the next pick.
    pub(crate) fn tonight(&self) -> Option<&Movie> {
        let pool: Vec<&Movie> = if self.movies.iter().any(|m| m.poster_id.is_some()) {
            self.movies
                .iter()
                .filter(|m| m.poster_id.is_some())
                .collect()
        } else {
            self.movies.iter().collect()
        };
        if pool.is_empty() {
            return None;
        }
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() / 86_400)
            .unwrap_or(0);
        let i = pick(day, self.tonight_skip, pool.len());
        Some(pool[i])
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

    pub(crate) fn movie_view<'a>(&'a self, m: &'a Movie) -> Element<'a, Message> {
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
            row![
                chip("Play", Message::Watch(false, m.id)),
                Space::with_width(8),
                chip("Find subtitles", Message::SubsFind(false, m.id)),
                Space::with_width(8),
                chip("Back", Message::Back)
            ],
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
        if self.subs_for == Some((false, m.id)) {
            info = info.push(Space::with_height(12)).push(self.subs_panel());
        }
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

    pub(crate) fn show_view<'a>(
        &'a self,
        s: &'a Show,
        episodes: &'a [Episode],
    ) -> Element<'a, Message> {
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
            row![
                chip(
                    "Play",
                    Message::Watch(true, episodes.first().map_or(0, |e| e.id))
                ),
                Space::with_width(8),
                chip("Back", Message::Back)
            ],
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
                mouse_area(
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
                            Space::with_width(12),
                            button(text("subs").size(11).color(alpha(CREAM, 0.7)))
                                .style(style::plain)
                                .padding([2, 6])
                                .on_press(Message::SubsFind(true, e.id)),
                            Space::with_width(12),
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
                )
                .on_press(Message::Watch(true, e.id))
                .interaction(iced::mouse::Interaction::Pointer),
            );
            if self.subs_for == Some((true, e.id)) {
                info = info.push(self.subs_panel()).push(Space::with_height(8));
            }
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

impl App {
    /// The subtitle search for the open film or episode: a line on how it
    /// went, then the hits to pick from (in sync first, then the most used).
    fn subs_panel(&self) -> Element<'_, Message> {
        let mut col = column![].spacing(2);
        if let Some(n) = &self.subs_note {
            col = col.push(text(n.clone()).size(13).color(alpha(CREAM, 0.85)));
        }
        for (i, h) in self.subs_hits.iter().take(10).enumerate() {
            let mut what = vec![h.language.clone()];
            if h.exact {
                what.push("made for this file".into());
            }
            if h.hearing_impaired {
                what.push("hearing impaired".into());
            }
            what.push(format!("{} downloads", h.downloads));
            col = col.push(
                button(
                    row![
                        container(
                            text(if h.release.is_empty() {
                                "untitled".into()
                            } else {
                                h.release.clone()
                            })
                            .size(14)
                            .wrapping(text::Wrapping::None)
                        )
                        .width(Length::Fill)
                        .clip(true),
                        Space::with_width(12),
                        text(what.join(" · ")).size(12).color(alpha(CREAM, 0.7)),
                    ]
                    .align_y(Alignment::Center),
                )
                .style(style::plain)
                .padding([5, 0])
                .width(Length::Fill)
                .on_press(Message::SubsPick(i)),
            );
        }
        col = col
            .push(Space::with_height(4))
            .push(chip("Close", Message::SubsClose));
        container(col).padding([8, 0]).into()
    }
}

/// The pick for a day: spread over the list, the same all day, and every
/// "another" a different film until the list runs out.
fn pick(day: u64, skip: usize, n: usize) -> usize {
    let mut z = day.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z % n as u64) as usize + skip) % n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonight_is_the_same_all_day_and_another_moves_on() {
        assert_eq!(pick(20_000, 0, 50), pick(20_000, 0, 50));
        let seen: std::collections::HashSet<usize> = (0..50).map(|s| pick(20_000, s, 50)).collect();
        assert_eq!(seen.len(), 50);
        let days: std::collections::HashSet<usize> =
            (20_000..20_030).map(|d| pick(d, 0, 50)).collect();
        assert!(days.len() > 15, "{}", days.len());
    }

    #[test]
    fn english_is_named_first() {
        assert_eq!(
            subs_line(&["en".into(), "hr".into()]),
            "subtitles: English, Croatian"
        );
        assert_eq!(subs_line(&[]), "no subtitles");
    }
}
