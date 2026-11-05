//! A CRT over the whole window: dark scanlines every few pixels and a soft
//! vignette towards the edges. Drawn once per window size and cached, it
//! takes no input, so everything under it works as before. `c` turns it off
//! and on.

use iced::mouse;
use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Program};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

/// Pixels from one scanline to the next.
const PITCH: f32 = 3.0;
const LINE_ALPHA: f32 = 0.22;
/// How far the vignette reaches in from each edge, as a share of the side.
const VIGNETTE: f32 = 0.18;
const VIGNETTE_ALPHA: f32 = 0.45;

#[derive(Default)]
pub struct Crt {
    cache: Cache,
}

pub fn overlay<'a, M: 'a>(crt: &'a Crt) -> Element<'a, M> {
    Canvas::new(crt)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

impl<M> Program<M> for Crt {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        _event: canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<M>) {
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let g = self
            .cache
            .draw(renderer, bounds.size(), |frame: &mut Frame| {
                let w = bounds.width.floor();
                let h = bounds.height.floor();
                let line = Color::from_rgba(0.0, 0.0, 0.0, LINE_ALPHA);
                let mut y = 0.0;
                while y < h {
                    frame.fill_rectangle(Point::new(0.0, y), Size::new(w, 1.0), line);
                    y += PITCH;
                }
                // The vignette as thin bands, darker towards each edge. Whole
                // pixels only, for the software renderer.
                let reach = (w.min(h) * VIGNETTE).round().max(1.0);
                for i in 0..reach as i32 {
                    let t = 1.0 - i as f32 / reach;
                    let c = Color::from_rgba(0.0, 0.0, 0.0, VIGNETTE_ALPHA * t * t * t);
                    let i = i as f32;
                    frame.fill_rectangle(Point::new(0.0, i), Size::new(w, 1.0), c);
                    frame.fill_rectangle(Point::new(0.0, h - 1.0 - i), Size::new(w, 1.0), c);
                    frame.fill_rectangle(Point::new(i, 0.0), Size::new(1.0, h), c);
                    frame.fill_rectangle(Point::new(w - 1.0 - i, 0.0), Size::new(1.0, h), c);
                }
            });
        vec![g]
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        // Say nothing, so the widget under the pointer chooses the cursor.
        mouse::Interaction::None
    }
}
