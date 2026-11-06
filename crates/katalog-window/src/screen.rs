//! Screen: the home for films and shows, in the same green as Music. The set
//! big on the left with its keys and the guide under it; tonight's film and
//! what is new on the right.

use crate::api::fmt_ms;
use crate::home::{label, mono, rule};
use crate::tv::On;
use crate::video::{genres, length};
use crate::{style, App, Message};
use iced::widget::{
    button, column, container, horizontal_space, mouse_area, row, text, vertical_space, Space,
};
use iced::{Alignment, Color, Element, Length};
use style::{MONO_MEDIUM, PHOSPHOR as G, PHOSPHOR_DIM as DIM, SCREEN};

/// A key on the set, lit when it is the one in use.
pub(crate) fn key<'a>(l: &str, lit: bool, m: Message) -> Element<'a, Message> {
    button(
        container(mono(l, 11.0, if lit { SCREEN } else { G }).font(MONO_MEDIUM))
            .center_y(Length::Fixed(28.0))
            .padding([0, 10]),
    )
    .style(move |t, s| {
        let mut st = style::screen_key(t, s);
        if lit {
            st.background = Some(G.into());
            st.border.color = G;
        }
        st
    })
    .padding(0)
    .on_press(m)
    .into()
}

/// Minutes left, as the guide says it.
fn left(ms: i64) -> String {
    let min = (ms.max(0) + 59_999) / 60_000;
    if min >= 60 {
        format!("{}:{:02} left", min / 60, min % 60)
    } else {
        format!("{min} min left")
    }
}

impl App {
    pub(crate) fn screen_view(&self) -> Element<'_, Message> {
        let inner_w = self.width - 80.0;
        let inner_h = self.height - 64.0;
        // The set as big as fits beside the right column, with room under it
        // for its keys and the guide.
        let w = (inner_w - 40.0 - 400.0)
            .min((inner_h - 70.0 - 250.0) * 16.0 / 9.0)
            .clamp(320.0, 1280.0)
            .floor();
        let top = row![
            container(self.screen_set(w)).width(Length::Fixed(w)),
            container(self.screen_side()).width(Length::Fill),
        ]
        .spacing(40)
        .height(Length::Fill);
        container(
            column![self.home_header(), top]
                .spacing(24)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .padding([32, 40])
        // A shade apart after the set moves (see `Tv::repaint`): the whole
        // page counts as changed and is drawn again, also where mpv was.
        .style(style::filled_text(
            if self.tv.repaint.is_multiple_of(2) {
                SCREEN
            } else {
                Color::from_rgb8(5, 5, 6)
            },
            G,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// The set, the line under it, its keys and the guide.
    fn screen_set(&self, w: f32) -> Element<'_, Message> {
        let h = (w * 9.0 / 16.0).round();
        let ch = self.channels.get(self.tv.channel);
        let now = ch.and_then(|c| c.now.as_ref());
        // Under mpv's picture iced draws black; without one, a card.
        let picture: Element<Message> = if self.tv.embedded() {
            container(Space::new(Length::Fill, Length::Fill))
                .style(style::filled(Color::BLACK))
                .into()
        } else {
            let (title, sub) = match (&self.tv.on, now) {
                _ if !self.tv.power => ("OFF".to_string(), "TURN ON below".to_string()),
                (On::Asked(t), _) => (t.clone(), String::new()),
                (_, Some(s)) => (s.title.clone(), s.subtitle.clone().unwrap_or_default()),
                _ => (
                    "NO SIGNAL".to_string(),
                    "nothing with a known length yet".into(),
                ),
            };
            let hint = if self.tv.power && now.is_some() {
                if self.tv.standalone || self.tv.parent.is_none() {
                    "click to watch"
                } else {
                    "tuning in"
                }
            } else {
                ""
            };
            container(
                column![
                    mono(title, 22.0, G).font(MONO_MEDIUM),
                    mono(sub, 12.0, DIM),
                    vertical_space(),
                    mono(hint, 12.0, DIM),
                ]
                .height(Length::Fill),
            )
            .padding(20)
            .style(style::filled(Color::BLACK))
            .into()
        };
        let set = crate::tv::screen(
            container(picture)
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .style(style::outlined(style::PHOSPHOR_LINE)),
            Message::TvArea,
            Message::TvClick,
        );
        let line = match (&self.tv.on, now) {
            _ if !self.tv.power => String::new(),
            _ if self.tv.note.is_some() => self.tv.note.clone().unwrap_or_default(),
            (On::Asked(t), _) => format!("playing · {t}"),
            (_, Some(s)) => {
                let mut l = s.title.clone();
                if let Some(sub) = &s.subtitle {
                    l.push_str(&format!(" · {sub}"));
                }
                l
            }
            _ => String::new(),
        };
        let when = match now {
            Some(s) if self.tv.power && !matches!(self.tv.on, On::Asked(_)) => {
                format!("{} / {}", fmt_ms(s.position_ms), fmt_ms(s.duration_ms))
            }
            _ => String::new(),
        };
        let mut keys = row![].spacing(6).align_y(Alignment::Center);
        for (i, c) in self.channels.iter().enumerate() {
            let lit = self.tv.power && i == self.tv.channel && !matches!(self.tv.on, On::Asked(_));
            keys = keys.push(key(
                &format!("{} {}", c.number, c.name.to_uppercase()),
                lit,
                Message::TvChannel(i),
            ));
        }
        keys = keys
            .push(Space::with_width(12))
            .push(key("SUB", false, Message::TvCycle("sub")))
            .push(key("AUDIO", false, Message::TvCycle("audio")))
            .push(horizontal_space())
            .push(key(
                if self.tv.power { "TURN OFF" } else { "TURN ON" },
                false,
                Message::TvPower,
            ));
        column![
            set,
            Space::with_height(10),
            row![
                container(mono(line, 14.0, G).wrapping(text::Wrapping::None))
                    .width(Length::Fill)
                    .clip(true),
                mono(when, 12.0, DIM),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            Space::with_height(10),
            keys,
            Space::with_height(20),
            self.guide(),
        ]
        .width(Length::Fixed(w))
        .into()
    }

    /// What is on each channel now and next. A row tunes to its channel.
    fn guide(&self) -> Element<'_, Message> {
        let mut rows = column![
            rule(),
            Space::with_height(12),
            row![
                container(label("Channel")).width(Length::Fixed(150.0)),
                container(label("Now")).width(Length::FillPortion(3)),
                container(label("Next")).width(Length::FillPortion(2)),
            ],
            Space::with_height(4),
        ]
        .spacing(4);
        for (i, c) in self.channels.iter().enumerate() {
            let on = self.tv.power && i == self.tv.channel && !matches!(self.tv.on, On::Asked(_));
            let color = if on { G } else { DIM };
            let now = c
                .now
                .as_ref()
                .map(|s| {
                    let mut t = s.title.clone();
                    if let Some(sub) = &s.subtitle {
                        // "S01E03 · Zen": the code is enough here.
                        t.push(' ');
                        t.push_str(sub.split(" · ").next().unwrap_or(sub));
                    }
                    format!("{t} · {}", left(s.duration_ms - s.position_ms))
                })
                .unwrap_or_else(|| "nothing on".into());
            let next = c
                .next
                .as_ref()
                .map(|s| match &s.subtitle {
                    Some(sub) => format!("{} {}", s.title, sub.split(" · ").next().unwrap_or(sub)),
                    None => s.title.clone(),
                })
                .unwrap_or_default();
            let cell = |s: String, c: Color| {
                container(mono(s, 13.0, c).wrapping(text::Wrapping::None)).clip(true)
            };
            rows = rows.push(
                button(
                    row![
                        cell(format!("{} {}", c.number, c.name), color).width(Length::Fixed(150.0)),
                        cell(now, color).width(Length::FillPortion(3)),
                        cell(next, DIM).width(Length::FillPortion(2)),
                    ]
                    .spacing(8),
                )
                .style(style::screen_link(color))
                .padding([3, 0])
                .on_press(Message::TvChannel(i)),
            );
        }
        rows.into()
    }

    /// Tonight's film and what is new.
    fn screen_side(&self) -> Element<'_, Message> {
        let mut side = column![
            row![label("Tonight's film"), horizontal_space()],
            Space::with_height(8),
            rule(),
            Space::with_height(16),
        ];
        match self.tonight() {
            Some(m) => {
                let mut meta = vec![];
                if let Some(y) = m.year {
                    meta.push(y.to_string());
                }
                if m.duration_ms > 0 {
                    meta.push(length(m.duration_ms));
                }
                if !m.genres.is_empty() {
                    meta.push(genres(&m.genres).to_lowercase());
                }
                let mut info = column![
                    mono(
                        m.title.clone(),
                        if m.title.len() > 22 { 20.0 } else { 26.0 },
                        G
                    )
                    .font(MONO_MEDIUM),
                    Space::with_height(6),
                    mono(meta.join(" · "), 12.0, DIM),
                    Space::with_height(10),
                ];
                if let Some(o) = &m.overview {
                    // A taste, not the whole blurb.
                    let short: String = if o.chars().count() > 160 {
                        let cut: String = o.chars().take(160).collect();
                        format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a))
                    } else {
                        o.clone()
                    };
                    info = info.push(mono(short, 12.0, DIM).line_height(1.5));
                }
                info = info.push(vertical_space()).push(
                    row![
                        key("PLAY NOW", true, Message::Watch(false, m.id)),
                        key("MORE", false, Message::OpenMovie(m.id)),
                        key("ANOTHER", false, Message::TonightNext),
                    ]
                    .spacing(6),
                );
                side = side.push(
                    row![
                        mouse_area(self.poster(m.poster_id, &m.title, m.id, 150.0))
                            .on_press(Message::OpenMovie(m.id))
                            .interaction(iced::mouse::Interaction::Pointer),
                        info.height(Length::Fixed(225.0)).width(Length::Fill),
                    ]
                    .spacing(18),
                );
            }
            None => {
                side = side.push(mono(
                    "No films yet. katalogd reads them from Movies/ next to Music/.",
                    12.0,
                    DIM,
                ));
            }
        }
        if !self.left_off.is_empty() {
            side = side
                .push(Space::with_height(32))
                .push(row![label("Where you left off"), horizontal_space()])
                .push(Space::with_height(8))
                .push(rule())
                .push(Space::with_height(8));
            for l in self.left_off.iter().take(3) {
                let episode = l.kind == "episode";
                let mut what = l.title.clone();
                if let Some(sub) = &l.subtitle {
                    what.push_str(&format!(" · {sub}"));
                }
                let at = if l.position_ms == 0 {
                    "next up".to_string()
                } else if l.duration_ms > 0 {
                    format!("{} of {}", fmt_ms(l.position_ms), fmt_ms(l.duration_ms))
                } else {
                    fmt_ms(l.position_ms)
                };
                let open = if episode {
                    Message::OpenShow(l.open_id)
                } else {
                    Message::OpenMovie(l.open_id)
                };
                side = side.push(
                    row![
                        button(
                            column![
                                mono(what, 14.0, G).wrapping(text::Wrapping::None),
                                mono(at, 11.0, DIM),
                            ]
                            .spacing(2)
                        )
                        .style(style::screen_link(G))
                        .padding(0)
                        .width(Length::Fill)
                        .on_press(open),
                        key("RESUME", false, Message::Watch(episode, l.id)),
                        key("×", false, Message::ForgetLeftOff(episode, l.id)),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .padding([4, 0]),
                );
            }
        }
        side = side
            .push(Space::with_height(32))
            .push(row![
                label("New"),
                horizontal_space(),
                button(mono("all in the catalogue", 11.0, DIM))
                    .style(style::screen_link(DIM))
                    .padding(0)
                    .on_press(Message::JumpTo(crate::Section::Films)),
            ])
            .push(Space::with_height(8))
            .push(rule())
            .push(Space::with_height(16));
        // The newest films and shows, by when katalogd first saw them.
        let mut films: Vec<_> = self.movies.iter().collect();
        films.sort_by_key(|m| std::cmp::Reverse(m.id));
        let mut shows: Vec<_> = self.shows.iter().collect();
        shows.sort_by_key(|s| std::cmp::Reverse(s.id));
        let mut fresh: Vec<Element<Message>> = vec![];
        let (mut f, mut s) = (films.into_iter(), shows.into_iter());
        let pw = ((self.width - 80.0) * 0.3 / 3.0 - 12.0)
            .clamp(90.0, 150.0)
            .floor();
        while fresh.len() < 3 {
            let mut any = false;
            if let Some(m) = f.next() {
                any = true;
                fresh.push(self.fresh_card(
                    m.poster_id,
                    &m.title,
                    m.id,
                    format!(
                        "film · {}",
                        m.year.map(|y| y.to_string()).unwrap_or_default()
                    ),
                    Message::OpenMovie(m.id),
                    pw,
                ));
            }
            if fresh.len() < 3 {
                if let Some(sh) = s.next() {
                    any = true;
                    fresh.push(self.fresh_card(
                        sh.poster_id,
                        &sh.title,
                        sh.id,
                        format!("show · {} episodes", sh.episode_count),
                        Message::OpenShow(sh.id),
                        pw,
                    ));
                }
            }
            if !any {
                break;
            }
        }
        side.push(row(fresh).spacing(12)).into()
    }

    fn fresh_card<'a>(
        &self,
        poster: Option<i64>,
        title: &str,
        seed: i64,
        meta: String,
        open: Message,
        w: f32,
    ) -> Element<'a, Message> {
        mouse_area(
            column![
                self.poster(poster, title, seed, w),
                Space::with_height(6),
                container(mono(title.to_string(), 12.0, G).wrapping(text::Wrapping::None))
                    .width(Length::Fixed(w))
                    .clip(true),
                mono(meta, 11.0, DIM).wrapping(text::Wrapping::None),
            ]
            .width(Length::Fixed(w)),
        )
        .on_press(open)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
    }
}
