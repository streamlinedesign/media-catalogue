//! The player: a queue of track ids, one decoder, one ALSA sink, on its own
//! thread. Views talk to it through `Player` (commands in, `Status` out).
//!
//! Tracks follow each other without a gap when they share a rate and depth;
//! when the format changes the sink is drained and reopened. Every start and
//! finish goes into the `plays` table.

pub mod decode;
pub mod output;

use anyhow::{Context, Result};
use decode::Decoder;
use katalog_core::db::{self, Track};
use output::Sink;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayState {
    Stopped,
    Playing,
    Paused,
}

/// What every view shows. Cheap to clone.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub state: PlayState,
    pub track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub queue: Vec<i64>,
    pub index: Option<usize>,
    /// 0.0 to 1.0. At 1.0 the samples are untouched.
    pub volume: f32,
    pub device: String,
    /// The stream as the card is running it, like "44100 Hz · 16 bit · 2 ch".
    pub format: Option<String>,
    /// True while an hw: device runs at the file's own rate and depth.
    pub bit_perfect: bool,
    /// The last thing that went wrong, until the next track starts.
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum Cmd {
    /// Replace the queue and start at `start`.
    Play {
        tracks: Vec<i64>,
        start: usize,
    },
    /// Append to the queue; start playing if stopped.
    Add {
        tracks: Vec<i64>,
    },
    Jump(usize),
    Remove(usize),
    Clear,
    Pause,
    Resume,
    Toggle,
    Stop,
    Next,
    Prev,
    SeekMs(u64),
    Volume(f32),
}

#[derive(Clone)]
pub struct Player {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
}

impl Player {
    /// Starts the player thread with its own database connection.
    pub fn start(db_path: &Path, device: String) -> Result<Self> {
        let conn = db::open(db_path).context("player: open database")?;
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status {
            state: PlayState::Stopped,
            track: None,
            position_ms: 0,
            duration_ms: 0,
            queue: Vec::new(),
            index: None,
            volume: 1.0,
            device: device.clone(),
            format: None,
            bit_perfect: false,
            error: None,
        }));
        let st = status.clone();
        std::thread::Builder::new()
            .name("player".into())
            .spawn(move || Engine::new(conn, device, rx, st).run())
            .context("spawn player thread")?;
        Ok(Self { tx, status })
    }

    pub fn send(&self, cmd: Cmd) -> Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| anyhow::anyhow!("player thread is gone"))
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

/// The thread's side.
struct Engine {
    conn: Connection,
    device: String,
    rx: Receiver<Cmd>,
    status: Arc<Mutex<Status>>,
    queue: Vec<i64>,
    index: Option<usize>,
    current: Option<Current>,
    sink: Option<Sink>,
    paused: bool,
    volume: f32,
    error: Option<String>,
}

struct Current {
    track: Track,
    decoder: Decoder,
    play_id: i64,
    /// Frames handed to the sink since the decoder's last seek.
    frames_out: u64,
    finished_marked: bool,
}

impl Engine {
    fn new(
        conn: Connection,
        device: String,
        rx: Receiver<Cmd>,
        status: Arc<Mutex<Status>>,
    ) -> Self {
        Self {
            conn,
            device,
            rx,
            status,
            queue: Vec::new(),
            index: None,
            current: None,
            sink: None,
            paused: false,
            volume: 1.0,
            error: None,
        }
    }

    fn run(mut self) {
        loop {
            let active = self.current.is_some() && !self.paused;
            let cmd = if active {
                match self.rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                // Nothing to push to the card: wait for a command, but wake now
                // and then so a paused status still shows the right position.
                match self.rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some(cmd) = cmd {
                if let Err(e) = self.handle(cmd) {
                    error!("player: {e:#}");
                    self.error = Some(format!("{e:#}"));
                }
            }
            if self.current.is_some() && !self.paused {
                if let Err(e) = self.pump() {
                    error!("player: {e:#}");
                    self.error = Some(format!("{e:#}"));
                    self.skip_broken();
                }
            }
            self.publish();
        }
    }

    fn handle(&mut self, cmd: Cmd) -> Result<()> {
        match cmd {
            Cmd::Play { tracks, start } => {
                if tracks.is_empty() {
                    anyhow::bail!("nothing to play");
                }
                self.finish_current(false);
                self.queue = tracks;
                self.paused = false;
                self.start_index(start.min(self.queue.len() - 1))?;
            }
            Cmd::Add { tracks } => {
                let was_empty = self.queue.is_empty();
                self.queue.extend(tracks);
                if was_empty || (self.current.is_none() && !self.queue.is_empty()) {
                    // Stopped at the end of the queue: carry on with what was added.
                    let next = match self.index {
                        Some(i) if i + 1 < self.queue.len() && !was_empty => i + 1,
                        _ => 0,
                    };
                    self.paused = false;
                    self.start_index(next)?;
                }
            }
            Cmd::Jump(i) => {
                if i >= self.queue.len() {
                    anyhow::bail!("no track {i} in the queue");
                }
                self.finish_current(false);
                self.paused = false;
                self.start_index(i)?;
            }
            Cmd::Remove(i) => {
                if i >= self.queue.len() {
                    anyhow::bail!("no track {i} in the queue");
                }
                if self.index == Some(i) {
                    self.finish_current(false);
                    self.queue.remove(i);
                    if i < self.queue.len() {
                        self.start_index(i)?;
                    } else {
                        self.index = None;
                        self.close_sink();
                    }
                } else {
                    self.queue.remove(i);
                    if let Some(cur) = self.index {
                        if cur > i {
                            self.index = Some(cur - 1);
                        }
                    }
                }
            }
            Cmd::Clear => {
                self.finish_current(false);
                self.queue.clear();
                self.index = None;
                self.close_sink();
            }
            Cmd::Pause => self.set_paused(true),
            Cmd::Resume => self.set_paused(false),
            Cmd::Toggle => {
                if self.current.is_some() {
                    let p = !self.paused;
                    self.set_paused(p);
                } else if !self.queue.is_empty() {
                    // Stopped with a queue: toggle starts it again from the top.
                    self.paused = false;
                    self.start_index(0)?;
                }
            }
            Cmd::Stop => {
                self.finish_current(false);
                self.close_sink();
                self.paused = false;
            }
            Cmd::Next => self.advance(1)?,
            Cmd::Prev => {
                // Early in a track, "previous" means the one before; later it restarts.
                let pos = self.position_ms();
                if pos > 3000 || self.index == Some(0) {
                    self.seek(0)?;
                } else {
                    self.advance(-1)?;
                }
            }
            Cmd::SeekMs(ms) => self.seek(ms)?,
            Cmd::Volume(v) => self.volume = v.clamp(0.0, 1.0),
        }
        Ok(())
    }

    fn start_index(&mut self, i: usize) -> Result<()> {
        let id = self.queue[i];
        let track = db::track(&self.conn, id)?.with_context(|| format!("track {id} is gone"))?;
        let decoder = Decoder::open(Path::new(&track.path))?;
        self.ensure_sink(decoder.spec)?;
        let play_id = self.record_start(id)?;
        info!(
            "playing {} · {} ({})",
            track.artist, track.title, decoder.spec
        );
        self.error = None;
        self.index = Some(i);
        self.current = Some(Current {
            track,
            decoder,
            play_id,
            frames_out: 0,
            finished_marked: false,
        });
        Ok(())
    }

    /// Moves by `by` in the queue. Past the end: stop and release the card.
    fn advance(&mut self, by: i64) -> Result<()> {
        let Some(cur) = self.index else {
            return Ok(());
        };
        let next = cur as i64 + by;
        self.finish_current(false);
        if next < 0 || next as usize >= self.queue.len() {
            if let Some(s) = self.sink.as_mut() {
                s.drain();
            }
            self.close_sink();
            self.index = None;
            info!("end of queue");
            return Ok(());
        }
        self.paused = false;
        self.start_index(next as usize)
    }

    /// Decodes one block and hands it to the card. Steps to the next track at the end.
    fn pump(&mut self) -> Result<()> {
        let Some(cur) = self.current.as_mut() else {
            return Ok(());
        };
        let chunk = cur.decoder.next()?;
        match chunk {
            Some(chunk) => {
                let spec = chunk.spec;
                let frames = chunk.samples.len() / spec.channels as usize;
                let samples: std::borrow::Cow<[i32]> = if self.volume < 0.999 {
                    let g = (self.volume * self.volume * 65536.0) as i64;
                    std::borrow::Cow::Owned(
                        chunk
                            .samples
                            .iter()
                            .map(|s| ((*s as i64 * g) >> 16) as i32)
                            .collect(),
                    )
                } else {
                    std::borrow::Cow::Borrowed(chunk.samples)
                };
                // The borrow of `cur` ends here so the sink can be touched.
                let samples = samples.into_owned();
                let need_reopen = self.sink.as_ref().map(|s| s.spec != spec).unwrap_or(true);
                if need_reopen {
                    if let Some(s) = self.sink.as_mut() {
                        s.drain();
                    }
                    self.sink = None;
                    self.ensure_sink(spec)?;
                }
                self.sink.as_mut().unwrap().write(&samples)?;
                if let Some(cur) = self.current.as_mut() {
                    cur.frames_out += frames as u64;
                    if !cur.finished_marked && cur.track.duration_ms > 0 {
                        // Half way through counts as a play, like a scrobble would.
                        let pos = cur.frames_out * 1000 / spec.rate as u64;
                        if pos * 2 >= cur.track.duration_ms as u64 {
                            cur.finished_marked = true;
                            let _ = mark_finished(&self.conn, cur.play_id);
                        }
                    }
                }
                Ok(())
            }
            None => {
                // End of file: the next track starts straight away, into the
                // same open sink when the format matches, so there is no gap.
                self.finish_current(true);
                let Some(cur) = self.index else {
                    return Ok(());
                };
                if cur + 1 < self.queue.len() {
                    self.start_index(cur + 1)
                } else {
                    if let Some(s) = self.sink.as_mut() {
                        s.drain();
                    }
                    self.close_sink();
                    self.index = None;
                    info!("end of queue");
                    Ok(())
                }
            }
        }
    }

    fn ensure_sink(&mut self, spec: decode::Spec) -> Result<()> {
        match &self.sink {
            Some(s) if s.spec == spec => Ok(()),
            _ => {
                if let Some(s) = self.sink.as_mut() {
                    s.drain();
                }
                self.sink = None;
                let sink = match Sink::open(&self.device, spec) {
                    Ok(s) => s,
                    Err(e) if self.device != "default" => {
                        warn!("{e:#}; falling back to the ALSA default device");
                        Sink::open("default", spec)?
                    }
                    Err(e) => return Err(e),
                };
                self.sink = Some(sink);
                Ok(())
            }
        }
    }

    fn close_sink(&mut self) {
        self.sink = None;
    }

    fn set_paused(&mut self, paused: bool) {
        if self.current.is_none() || self.paused == paused {
            return;
        }
        self.paused = paused;
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        if paused {
            if sink.pause_drops() {
                // What was buffered is lost; wind the decoder back to match.
                let lost = sink.delay_frames();
                sink.pause();
                if let Some(cur) = self.current.as_mut() {
                    let ms =
                        cur.frames_out.saturating_sub(lost) * 1000 / cur.decoder.spec.rate as u64;
                    if let Ok(landed) = cur.decoder.seek(ms) {
                        cur.frames_out = landed * cur.decoder.spec.rate as u64 / 1000;
                    }
                }
            } else {
                sink.pause();
            }
        } else {
            sink.resume();
        }
    }

    fn seek(&mut self, ms: u64) -> Result<()> {
        let Some(cur) = self.current.as_mut() else {
            return Ok(());
        };
        let landed = cur.decoder.seek(ms)?;
        cur.frames_out = landed * cur.decoder.spec.rate as u64 / 1000;
        if let Some(s) = self.sink.as_mut() {
            s.flush();
            if self.paused {
                // Keep the card quiet until resume.
                s.pause();
            }
        }
        Ok(())
    }

    fn position_ms(&self) -> u64 {
        let Some(cur) = self.current.as_ref() else {
            return 0;
        };
        let delay = self.sink.as_ref().map(|s| s.delay_frames()).unwrap_or(0);
        cur.frames_out.saturating_sub(delay) * 1000 / cur.decoder.spec.rate.max(1) as u64
    }

    /// Closes the current track. `finished` marks the play as heard to the end.
    fn finish_current(&mut self, finished: bool) {
        if let Some(cur) = self.current.take() {
            if finished && !cur.finished_marked {
                let _ = mark_finished(&self.conn, cur.play_id);
            }
        }
    }

    /// A file that cannot be decoded or a card error: move on, do not loop on it.
    fn skip_broken(&mut self) {
        let Some(cur) = self.index else {
            self.current = None;
            return;
        };
        if let Some(c) = &self.current {
            warn!("skipping {} ({})", c.track.title, c.track.path);
        }
        self.current = None;
        self.sink = None;
        if cur + 1 < self.queue.len() {
            if let Err(e) = self.start_index(cur + 1) {
                error!("player: {e:#}");
                self.index = None;
            }
        } else {
            self.index = None;
        }
    }

    fn record_start(&self, track_id: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO plays (track_id, started_at, finished, source) VALUES (?1, ?2, 0, 'manual')",
            rusqlite::params![track_id, db::now_iso()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn publish(&self) {
        let state = match (&self.current, self.paused) {
            (None, _) => PlayState::Stopped,
            (Some(_), true) => PlayState::Paused,
            (Some(_), false) => PlayState::Playing,
        };
        let mut st = self.status.lock().unwrap_or_else(|p| p.into_inner());
        st.state = state;
        st.track = self.current.as_ref().map(|c| c.track.clone());
        st.position_ms = self.position_ms();
        st.duration_ms = self
            .current
            .as_ref()
            .map(|c| c.track.duration_ms.max(0) as u64)
            .unwrap_or(0);
        if st.queue != self.queue {
            st.queue = self.queue.clone();
        }
        st.index = self.index;
        st.volume = self.volume;
        st.device = self
            .sink
            .as_ref()
            .map(|s| s.device.clone())
            .unwrap_or_else(|| self.device.clone());
        st.format = self.sink.as_ref().map(|s| s.spec.to_string());
        st.bit_perfect =
            self.sink.as_ref().map(|s| s.bit_perfect).unwrap_or(false) && self.volume >= 0.999;
        st.error = self.error.clone();
    }
}

fn mark_finished(conn: &Connection, play_id: i64) -> Result<()> {
    conn.execute("UPDATE plays SET finished = 1 WHERE id = ?1", [play_id])?;
    Ok(())
}
