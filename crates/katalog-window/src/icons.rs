//! Transport glyphs and the progress line, drawn rather than typed, so they
//! match the rules and hairlines of the rest of the window.

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Program};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Play,
    Pause,
    Prev,
    Next,
}

pub struct Icon {
    pub glyph: Glyph,
    pub color: Color,
}

pub fn icon<'a, M: 'a>(glyph: Glyph, color: Color, size: f32) -> Element<'a, M> {
    Canvas::new(Icon { glyph, color })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

impl<M> Program<M> for Icon {
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
        let w = bounds.width;
        let h = bounds.height;
        let c = self.color;
        // Whole pixels only: the software renderer drops shapes on fractional coordinates.
        let pt = |x: f32, y: f32| Point::new(x.round(), y.round());
        let sz = |x: f32, y: f32| Size::new(x.round().max(1.0), y.round().max(1.0));
        match self.glyph {
            Glyph::Play => {
                // A triangle, optically centred (shifted a touch right).
                let p = Path::new(|b| {
                    b.move_to(pt(w * 0.22, h * 0.12));
                    b.line_to(pt(w * 0.88, h * 0.5));
                    b.line_to(pt(w * 0.22, h * 0.88));
                    b.close();
                });
                frame.fill(&p, c);
            }
            Glyph::Pause => {
                let bar = (w * 0.22).round();
                frame.fill_rectangle(pt(w * 0.2, h * 0.14), sz(bar, h * 0.72), c);
                frame.fill_rectangle(pt(w * 0.8 - bar, h * 0.14), sz(bar, h * 0.72), c);
            }
            Glyph::Next => {
                let p = Path::new(|b| {
                    b.move_to(pt(w * 0.12, h * 0.16));
                    b.line_to(pt(w * 0.72, h * 0.5));
                    b.line_to(pt(w * 0.12, h * 0.84));
                    b.close();
                });
                frame.fill(&p, c);
                frame.fill_rectangle(pt(w * 0.76, h * 0.16), sz(w * 0.12, h * 0.68), c);
            }
            Glyph::Prev => {
                let p = Path::new(|b| {
                    b.move_to(pt(w * 0.88, h * 0.16));
                    b.line_to(pt(w * 0.28, h * 0.5));
                    b.line_to(pt(w * 0.88, h * 0.84));
                    b.close();
                });
                frame.fill(&p, c);
                frame.fill_rectangle(pt(w * 0.12, h * 0.16), sz(w * 0.12, h * 0.68), c);
            }
        }
        vec![frame.into_geometry()]
    }
}

/// The progress line: a hairline with the played part drawn over it.
/// Clicking anywhere on it seeks there.
pub struct Progress<M> {
    pub fraction: f32,
    pub line: Color,
    pub fill: Color,
    pub on_seek: fn(f32) -> M,
}

pub fn progress<'a, M: 'a>(
    fraction: f32,
    line: Color,
    fill: Color,
    on_seek: fn(f32) -> M,
) -> Element<'a, M> {
    Canvas::new(Progress {
        fraction,
        line,
        fill,
        on_seek,
    })
    .width(Length::Fill)
    .height(Length::Fixed(14.0))
    .into()
}

impl<M> Program<M> for Progress<M> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<M>) {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(p) = cursor.position_in(bounds) {
                let f = (p.x / bounds.width).clamp(0.0, 1.0);
                return (canvas::event::Status::Captured, Some((self.on_seek)(f)));
            }
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width.round();
        let y = (bounds.height / 2.0).round();
        frame.fill_rectangle(Point::new(0.0, y), Size::new(w, 1.0), self.line);
        let played = (w * self.fraction.clamp(0.0, 1.0)).round();
        if played >= 1.0 {
            frame.fill_rectangle(Point::new(0.0, y - 1.0), Size::new(played, 2.0), self.fill);
        }
        // A hover mark where a click would land.
        if let Some(p) = cursor.position_in(bounds) {
            frame.fill_rectangle(
                Point::new(p.x - 0.5, y - 5.0),
                Size::new(1.0, 10.0),
                self.fill,
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
