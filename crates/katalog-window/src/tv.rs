//! The television on Home: three channels on a schedule, and whatever film or
//! episode was last played, shown by mpv inside the window.
//!
//! On X11 the set is a child window of Katalog's own, and mpv draws into it
//! with `--wid`. The set takes no input, so a click on the picture reaches
//! the window like a click anywhere else: one click makes it
//! fullscreen, another (or Esc) brings it back. Where there is no X11 the
//! set shows what is on, and playing opens mpv in a window of its own.
//!
//! On Home the set plays its channel without sound; fullscreen, or a film
//! you asked for, has sound. The radio pauses for either.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, tree, Tree, Widget};
use iced::advanced::{renderer, Clipboard, Shell};
use iced::{event, mouse, Element, Event, Length, Rectangle, Size};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use x11rb::connection::Connection as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    ClipOrdering, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, StackMode, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// A window inside Katalog's, for mpv to draw in.
struct Embed {
    conn: RustConnection,
    win: u32,
    shown: Option<(i32, i32, u32, u32)>,
}

impl Embed {
    fn new(parent: u32) -> anyhow::Result<Self> {
        let (conn, _) = x11rb::connect(None)?;
        let win = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            parent,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            // Black until mpv draws.
            &CreateWindowAux::new().background_pixel(0),
        )?;
        // No input region: the pointer passes through the set, and mpv's
        // window inside it, as if they were not there. Katalog keeps the
        // cursor and sees the click on the picture like any other.
        conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            win,
            0,
            0,
            &[],
        )?;
        conn.flush()?;
        Ok(Self {
            conn,
            win,
            shown: None,
        })
    }

    fn place(&mut self, x: i32, y: i32, w: u32, h: u32) {
        let want = Some((x, y, w.max(1), h.max(1)));
        if self.shown == want {
            return;
        }
        let aux = ConfigureWindowAux::new()
            .x(x)
            .y(y)
            .width(w.max(1))
            .height(h.max(1))
            .stack_mode(StackMode::ABOVE);
        let _ = self.conn.configure_window(self.win, &aux);
        let _ = self.conn.map_window(self.win);
        let _ = self.conn.flush();
        self.shown = want;
    }

    fn hide(&mut self) {
        if self.shown.take().is_some() {
            let _ = self.conn.unmap_window(self.win);
            let _ = self.conn.flush();
        }
    }
}

impl Drop for Embed {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.win);
        let _ = self.conn.flush();
    }
}

/// What the set is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum On {
    Nothing,
    /// Channel n (0 to 2), the slot that started at this time.
    Channel(usize, i64),
    /// A film or episode asked for; the channel comes back when it ends.
    Asked(String),
}

pub struct Tv {
    /// Katalog's own X11 window, once known.
    pub parent: Option<u32>,
    pub scale: f32,
    embed: Option<Embed>,
    /// Could not make a window for mpv (no X11): mpv gets its own.
    pub standalone: bool,
    mpv: Option<Child>,
    socket: PathBuf,
    pub channel: usize,
    pub on: On,
    pub power: bool,
    /// Whether mpv has the sound card. Silent, it has no audio track at all,
    /// so the card stays free for the music.
    pub sound: bool,
    pub full: bool,
    /// Where the set is on Home, in logical pixels, when Home is showing.
    pub area: Option<Rectangle>,
    pub window: Size,
    /// Bumped when the picture leaves a place, so the page is drawn again
    /// there: the software renderer only redraws what it thinks changed.
    pub repaint: u8,
}

impl Default for Tv {
    fn default() -> Self {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self {
            parent: None,
            scale: 1.0,
            embed: None,
            standalone: std::env::var_os("DISPLAY").is_none(),
            mpv: None,
            socket: dir.join(format!("katalog-tv-{}.sock", std::process::id())),
            channel: 0,
            on: On::Nothing,
            power: std::env::var("KATALOG_TV")
                .map(|v| v != "0")
                .unwrap_or_else(|_| !off_marker().exists()),
            sound: false,
            full: false,
            area: None,
            window: Size::new(1440.0, 900.0),
            repaint: 0,
        }
    }
}

impl Tv {
    /// Whether mpv is still running.
    pub fn playing(&mut self) -> bool {
        match &mut self.mpv {
            Some(c) => match c.try_wait() {
                Ok(None) => true,
                _ => {
                    self.mpv = None;
                    false
                }
            },
            None => false,
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.mpv.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.sound = false;
        self.on = On::Nothing;
    }

    fn ensure_embed(&mut self) -> Option<u32> {
        if self.standalone {
            return None;
        }
        if self.embed.is_none() {
            let parent = self.parent?;
            match Embed::new(parent) {
                Ok(e) => self.embed = Some(e),
                Err(_) => {
                    self.standalone = true;
                    return None;
                }
            }
        }
        self.embed.as_ref().map(|e| e.win)
    }

    /// Puts the picture where it belongs: the whole window, the set on Home,
    /// or nowhere.
    pub fn place(&mut self) {
        let s = self.scale;
        let target = if self.full {
            Some(Rectangle::new(iced::Point::ORIGIN, self.window))
        } else {
            self.area
        };
        if target.is_some() {
            self.ensure_embed();
        }
        let Some(e) = self.embed.as_mut() else { return };
        let was = e.shown;
        match target {
            Some(r) if self.mpv.is_some() => e.place(
                (r.x * s).round() as i32,
                (r.y * s).round() as i32,
                (r.width * s).round() as u32,
                (r.height * s).round() as u32,
            ),
            _ => e.hide(),
        }
        if was.is_some() && e.shown != was {
            self.repaint = self.repaint.wrapping_add(1);
        }
    }

    /// Starts mpv on these arguments (from katalogd), with sound or not.
    pub fn play(&mut self, args: &[String], on: On, sound: bool) -> Result<(), String> {
        self.stop();
        let wid = self.ensure_embed();
        let mut cmd = Command::new("mpv");
        match wid {
            Some(w) => {
                cmd.arg(format!("--wid={w}"))
                    // Clicks and keys stay with Katalog.
                    .arg("--input-cursor=no")
                    .arg("--input-vo-keyboard=no")
                    .arg("--osc=no")
                    .arg("--cursor-autohide=no");
            }
            None => {
                cmd.arg("--force-window=immediate");
            }
        }
        cmd.arg(format!("--input-ipc-server={}", self.socket.display()))
            .arg("--keep-open=no")
            .arg("--idle=no")
            .args(args)
            // After katalogd's arguments, so this wins.
            .arg(format!("--aid={}", if sound { "auto" } else { "no" }))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // If Katalog dies without saying goodbye, mpv goes with it instead of
        // playing on in a window that no longer exists.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start mpv: {e}"))?;
        self.mpv = Some(child);
        self.sound = sound;
        self.on = on;
        self.place();
        Ok(())
    }

    /// Sends mpv a command over its socket, if it is listening.
    pub fn command(&self, cmd: serde_json::Value) {
        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(&self.socket) {
            let _ = writeln!(s, "{}", serde_json::json!({ "command": cmd }));
        }
    }

    pub fn set_pause(&self, on: bool) {
        self.command(serde_json::json!(["set_property", "pause", on]));
    }

    /// Gives mpv the sound card, or takes it back. Taking the audio track
    /// away closes mpv's audio output, which a mute would not.
    pub fn set_sound(&mut self, on: bool) {
        if self.sound == on {
            return;
        }
        self.sound = on;
        self.command(serde_json::json!([
            "set_property",
            "aid",
            if on { "auto" } else { "no" }
        ]));
    }

    /// Turns the set on or off, and remembers it for next time.
    pub fn set_power(&mut self, on: bool) {
        self.power = on;
        let marker = off_marker();
        if on {
            let _ = std::fs::remove_file(marker);
        } else {
            if let Some(d) = marker.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(marker, "");
        }
    }

    /// The set's picture is in mpv, not drawn by iced.
    pub fn embedded(&self) -> bool {
        self.embed.is_some() && self.mpv.is_some()
    }
}

/// Left behind when the set is switched off, so it stays off.
fn off_marker() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("katalog/tv-off")
}

impl Drop for Tv {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The place for the picture on Home. It tells the app where it is laid out
/// (mpv's window follows) and when it is clicked.
pub struct Screen<'a, Message> {
    content: Element<'a, Message>,
    on_area: fn(Rectangle) -> Message,
    on_click: Message,
}

pub fn screen<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    on_area: fn(Rectangle) -> Message,
    on_click: Message,
) -> Screen<'a, Message> {
    Screen {
        content: content.into(),
        on_area,
        on_click,
    }
}

#[derive(Default)]
struct Seen(Option<Rectangle>);

impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for Screen<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Seen>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Seen::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        let bounds = layout.bounds();
        if let Event::Window(iced::window::Event::RedrawRequested(_)) = &event {
            let seen = tree.state.downcast_mut::<Seen>();
            if seen.0 != Some(bounds) {
                seen.0 = Some(bounds);
                shell.publish((self.on_area)(bounds));
            }
        }
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = &event {
            if cursor.is_over(bounds) {
                shell.publish(self.on_click.clone());
                return event::Status::Captured;
            }
        }
        self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
}

impl<'a, Message: Clone + 'a> From<Screen<'a, Message>> for Element<'a, Message> {
    fn from(s: Screen<'a, Message>) -> Self {
        Element::new(s)
    }
}
