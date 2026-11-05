//! The Kaptol look: Satoshi, the palette, hairlines, and the widget styles
//! that keep iced's defaults out of the picture.

use iced::font::{Family, Weight};
use iced::widget::{button, container, rule, scrollable, text_input};
use iced::{Border, Color, Font, Theme};

pub const SATOSHI_REGULAR: &[u8] = include_bytes!("../fonts/Satoshi-Regular.ttf");
pub const SATOSHI_MEDIUM: &[u8] = include_bytes!("../fonts/Satoshi-Medium.ttf");
pub const SATOSHI_BOLD: &[u8] = include_bytes!("../fonts/Satoshi-Bold.ttf");
pub const SATOSHI_BLACK: &[u8] = include_bytes!("../fonts/Satoshi-Black.ttf");

pub const fn weight(w: Weight) -> Font {
    Font {
        family: Family::Name("Satoshi"),
        weight: w,
        stretch: iced::font::Stretch::Normal,
        style: iced::font::Style::Normal,
    }
}
pub const MEDIUM: Font = weight(Weight::Medium);
pub const BOLD: Font = weight(Weight::Bold);
pub const BLACK: Font = weight(Weight::Black);

const fn hex(rgb: u32) -> Color {
    Color::from_rgb(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    )
}

// Kaptol palette
pub const NAVY: Color = hex(0x1b395b);
pub const CREAM: Color = hex(0xfffabd);
pub const WHITE: Color = hex(0xfcfdff);
pub const WINE: Color = hex(0x2d0a1e);
pub const RED: Color = hex(0xe84a2d);
pub const OCHRE: Color = hex(0xd68c45);
pub const BROWN: Color = hex(0x3b2208);
pub const PINK: Color = hex(0xffb7c5);
pub const GREEN: Color = hex(0x2d7a3e);

// The black card (resend.com greys)
pub const CARD: Color = Color::BLACK;
pub const CARD_TEXT: Color = hex(0xededed);
pub const CARD_SOFT: Color = hex(0xa1a1a1);
pub const CARD_MUTE: Color = hex(0x707070);
pub const CARD_LINE: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.10);
pub const CARD_LINE_STRONG: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.18);

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Cover placeholders: background and foreground pairs from the palette.
pub fn placeholder_colors(seed: i64) -> (Color, Color, Color) {
    const SETS: [(Color, Color, Color); 6] = [
        (RED, CREAM, WINE),
        (OCHRE, BROWN, CREAM),
        (PINK, WINE, RED),
        (GREEN, CREAM, OCHRE),
        (WINE, CREAM, RED),
        (CREAM, NAVY, RED),
    ];
    SETS[(seed.unsigned_abs() % SETS.len() as u64) as usize]
}

pub fn theme() -> Theme {
    Theme::custom(
        "Katalog".into(),
        iced::theme::Palette {
            background: NAVY,
            text: CREAM,
            primary: CREAM,
            success: GREEN,
            danger: RED,
        },
    )
}

/// A flat button: no background, no border, the text as it is.
pub fn plain(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: CREAM,
        border: Border::default(),
        shadow: Default::default(),
    }
}

/// The same, but the text color follows the parent (for the black card).
pub fn plain_in(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: None,
        text_color: match status {
            button::Status::Hovered => WHITE,
            _ => color,
        },
        border: Border::default(),
        shadow: Default::default(),
    }
}

/// Outlined chip, like Kaptol's showtime buttons. Fills on hover.
pub fn chip(_theme: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { CREAM } else { Color::TRANSPARENT }.into()),
        text_color: if hovered { NAVY } else { CREAM },
        border: Border {
            color: CREAM,
            width: 1.0,
            radius: 0.0.into(),
        },
        shadow: Default::default(),
    }
}

/// Search field: no box, a rule underneath, big type.
pub fn search(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused);
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        icon: CREAM,
        placeholder: alpha(CREAM, if focused { 0.35 } else { 0.45 }),
        value: CREAM,
        selection: alpha(CREAM, 0.3),
    }
}

pub fn hairline(color: Color) -> impl Fn(&Theme) -> rule::Style {
    move |_| rule::Style {
        color,
        width: 1,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
    }
}

pub fn rule_px(color: Color, px: u16) -> impl Fn(&Theme) -> rule::Style {
    move |_| rule::Style {
        color,
        width: px,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
    }
}

pub fn filled(bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        ..Default::default()
    }
}

pub fn filled_text(bg: Color, fg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(bg.into()),
        text_color: Some(fg),
        ..Default::default()
    }
}

pub fn outlined(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        border: Border {
            color,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// A thin scroller in the text color, no rail background.
pub fn scroll(color: Color) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |_, status| {
        let active = matches!(
            status,
            scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. }
        );
        let rail = scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                color: alpha(color, if active { 0.6 } else { 0.25 }),
                border: Border::default(),
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
        }
    }
}

// ---- Home: green phosphor on black, IBM Plex Mono -------------------------

pub const PLEX_LIGHT: &[u8] = include_bytes!("../fonts/IBMPlexMono-Light.ttf");
pub const PLEX_REGULAR: &[u8] = include_bytes!("../fonts/IBMPlexMono-Regular.ttf");
pub const PLEX_MEDIUM: &[u8] = include_bytes!("../fonts/IBMPlexMono-Medium.ttf");

const fn plex(w: Weight) -> Font {
    Font {
        family: Family::Name("IBM Plex Mono"),
        weight: w,
        stretch: iced::font::Stretch::Normal,
        style: iced::font::Style::Normal,
    }
}
pub const MONO_LIGHT: Font = plex(Weight::Light);
pub const MONO: Font = plex(Weight::Normal);
pub const MONO_MEDIUM: Font = plex(Weight::Medium);

/// The screen.
pub const SCREEN: Color = hex(0x050505);
/// Lit phosphor: text that matters, bars, the needle.
pub const PHOSPHOR: Color = hex(0x4dff91);
/// Half lit: labels and secondary text.
pub const PHOSPHOR_DIM: Color = hex(0x3fa86a);
/// Barely lit: rules, borders, unlit segments.
pub const PHOSPHOR_LINE: Color = hex(0x163d26);

/// Text-only button on the screen: dim, lit on hover.
pub fn screen_link(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: None,
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => PHOSPHOR,
            _ => color,
        },
        border: Border::default(),
        shadow: Default::default(),
    }
}

/// A square key with a dim outline that lights on hover.
pub fn screen_key(_theme: &Theme, status: button::Status) -> button::Style {
    let lit = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            if matches!(status, button::Status::Pressed) {
                PHOSPHOR_LINE
            } else {
                Color::TRANSPARENT
            }
            .into(),
        ),
        text_color: PHOSPHOR,
        border: Border {
            color: if lit { PHOSPHOR } else { PHOSPHOR_LINE },
            width: 1.0,
            radius: 0.0.into(),
        },
        shadow: Default::default(),
    }
}
