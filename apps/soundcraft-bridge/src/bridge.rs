//! The bridge: an engine answering control-protocol methods without a window, and the audio
//! player its transport requests reach.
//!
//! The transport glue follows the app's (`ui-egui/src/lib.rs`, `handle_transport`, `start_play`,
//! `stop_play`): commands queue [`TransportRequest`]s on the engine, the bridge turns them into
//! [`Player`] calls after each command, and while playing it reads the player's position back
//! into `engine.transport`, so upstream's own `transport.status` command reports it too.

use serde_json::{Value, json};
use soundcraft_engine::{Engine, TransportRequest};
use soundcraft_playback::{OutputTrouble, Player};
use soundcraft_time::{Range, Samples};

/// An engine, its player, and what the protocol needs around them.
pub struct Bridge {
    pub engine: Engine,
    player: Player,
    /// The engine revision last sent to the player.
    sent_rev: u64,
    /// What stopped the audio output, once it has.
    trouble: Option<String>,
    quit: bool,
}

impl Bridge {
    /// A bridge playing through `player` (opened on `engine`'s session).
    pub fn new(engine: Engine, player: Player) -> Bridge {
        let sent_rev = engine.revision;
        Bridge { engine, player, sent_rev, trouble: None, quit: false }
    }

    /// A bridge on upstream's silent clock: the transport and the mix run in real time and no
    /// audio device is opened (`--no-audio`).
    pub fn silent(engine: Engine) -> Bridge {
        let player = Player::silent(engine.session_arc());
        Bridge::new(engine, player)
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    /// True once `bridge.quit` was answered.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Answer one control-protocol method.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.tick();
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str).ok_or("`command` required")?;
                let p = params.get("params").cloned().unwrap_or_else(|| json!({}));
                self.execute(id, &p)
            }
            "engine.commands" => self.engine.execute("engine.commands", &params).map_err(|e| e.to_string()),
            "engine.parity" => Ok(soundcraft_engine::catalog::parity_json()),
            "session.inspect" | "document.inspect" => {
                Ok(soundcraft_engine::inspect::session(&self.engine, params.get("detail").and_then(Value::as_str) == Some("full")))
            }
            "transport.status" => Ok(self.status()),
            // The transport commands as methods of their own: `transport.play {from?}` is
            // `engine.execute {command: "transport.play", params: {from?}}`.
            m if m.starts_with("transport.") => self.execute(m, &params),
            "bridge.quit" | "app.quit" => {
                self.quit = true;
                Ok(json!({}))
            }
            m if m.starts_with("ui.") => Err(format!("`{m}` needs the desktop app; the bridge has no window")),
            other => Err(format!("unknown method `{other}`")),
        }
    }

    /// The transport and the output device: position in session samples, read from the player.
    pub fn status(&mut self) -> Value {
        self.tick();
        let t = self.engine.transport;
        let rate = self.engine.session().sample_rate.hz();
        let p = &self.player;
        json!({
            "playing": t.playing,
            "recording": t.recording,
            "position": t.position,
            "seconds": if rate > 0 { t.position as f64 / f64::from(rate) } else { 0.0 },
            "sample_rate": rate,
            "speed": p.speed(),
            "device": {
                "name": p.device_name,
                "sample_rate": p.device_rate,
                "buffer": p.device_buffer,
                "channels": p.device_channels,
                "silent": p.silent,
            },
            "trouble": self.trouble,
        })
    }

    /// Follow the player: read the position back while playing, stop when playback ended or the
    /// output failed, hand a changed session to the audio thread, and drop retired plugins.
    pub fn tick(&mut self) {
        self.watch_output();
        if self.engine.transport.playing {
            let still = self.player.is_playing();
            self.engine.transport.position = self.player.position();
            if !still {
                self.stop_play();
            }
        }
        self.send_session();
        // Retired plugin instances and session snapshots are dropped here, off the audio thread.
        // The bridge opens no plugin editors, so there are no editor edits to apply.
        let _ = self.player.idle();
    }

    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let r = self.engine.execute(id, params).map_err(|e| e.to_string());
        let refused = self.handle_transport();
        self.send_session();
        match (r, refused) {
            (Ok(_), Some(e)) => Err(e),
            (r, _) => r,
        }
    }

    fn send_session(&mut self) {
        if self.engine.revision != self.sent_rev {
            self.sent_rev = self.engine.revision;
            self.player.update_session(self.engine.session_arc());
        }
    }

    fn watch_output(&mut self) {
        let Some(t) = self.player.trouble() else { return };
        let what = match t {
            OutputTrouble::Bug => "the audio engine hit an internal error".to_string(),
            OutputTrouble::Gone => format!("`{}` was disconnected", self.player.device_name),
            OutputTrouble::Stalled => format!("`{}` stopped asking for audio", self.player.device_name),
        };
        log::error!("audio output: {what}; the transport stopped");
        if self.engine.transport.playing {
            self.stop_play();
        }
        self.trouble = Some(what);
    }

    /// Turn the engine's queued transport requests into player calls. Returns the refusal of a
    /// request the bridge cannot serve.
    fn handle_transport(&mut self) -> Option<String> {
        let mut refused = None;
        for r in std::mem::take(&mut self.engine.transport_requests) {
            match r {
                TransportRequest::ResetSession => {
                    let p = &self.player;
                    p.stop();
                    p.set_recording(false);
                    p.set_speed(1.0);
                    p.locate(self.engine.transport.position);
                }
                TransportRequest::Play => {
                    if !self.engine.transport.playing {
                        self.play_from_selection();
                    }
                }
                TransportRequest::Stop | TransportRequest::Pause => self.stop_play(),
                TransportRequest::TogglePlay => {
                    if self.engine.transport.playing {
                        self.stop_play();
                    } else {
                        self.play_from_selection();
                    }
                }
                TransportRequest::Record => refused = Some("transport.record: the bridge does not record yet".to_string()),
                TransportRequest::Locate(at) => {
                    if self.engine.transport.playing {
                        self.player.locate(at);
                    }
                    self.engine.transport.position = at;
                }
                TransportRequest::PlaySelection => {
                    let sel = self.engine.session().edit.selection;
                    self.start_play(sel.start, Some(sel.end.max(sel.start + 1)), None);
                }
                TransportRequest::HalfSpeed => {
                    self.player.set_speed(0.5);
                    self.play_from_selection();
                }
                TransportRequest::Scrub(at) => self.engine.transport.position = at,
                TransportRequest::AllNotesOff => {}
            }
        }
        refused
    }

    /// Play from the selection, honouring loop playback, pre and post roll, and the selection end.
    fn play_from_selection(&mut self) {
        let s = self.engine.session();
        let sel = s.edit.selection;
        let preroll = if s.edit.pre_post_roll { s.edit.pre_roll } else { 0 };
        let postroll = if s.edit.pre_post_roll { s.edit.post_roll } else { 0 };
        if s.edit.loop_playback && !sel.is_empty() {
            self.start_play(sel.start, None, Some(sel));
        } else if !sel.is_empty() {
            self.start_play((sel.start - preroll).max(0), Some(sel.end + postroll), None);
        } else {
            self.start_play((sel.start - preroll).max(0), None, None);
        }
    }

    fn start_play(&mut self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.sent_rev = self.engine.revision;
        self.player.update_session(self.engine.session_arc());
        self.player.play(from, end, looped);
        self.engine.transport.playing = true;
        self.engine.transport.position = from;
    }

    fn stop_play(&mut self) {
        if self.engine.transport.playing {
            self.engine.transport.position = self.player.position();
        }
        // Half speed is a command of its own; Stop must not leave that rate for the next Play.
        self.player.set_speed(1.0);
        self.player.stop();
        self.engine.transport.playing = false;
        self.engine.transport.recording = false;
        if self.engine.session().edit.insertion_follows_playback {
            let pos = self.engine.transport.position;
            self.engine.session_mut().edit.selection = Range::point(pos);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn bridge() -> Bridge {
        let mut b = Bridge::silent(Engine::default());
        b.call("engine.execute", json!({"command": "session.new", "params": {"sample_rate": 48000}})).unwrap();
        b
    }

    /// Call `transport.status` until `done` holds (or two seconds pass).
    fn wait(b: &mut Bridge, done: impl Fn(&Value) -> bool) -> Value {
        let t0 = Instant::now();
        loop {
            let s = b.status();
            if done(&s) || t0.elapsed() > Duration::from_secs(2) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn play_locate_and_stop_reach_the_silent_clock() {
        let mut b = bridge();
        let s = b.status();
        assert_eq!((s["playing"].as_bool(), s["position"].as_i64()), (Some(false), Some(0)), "{s}");
        assert_eq!(s["device"]["silent"], true);

        b.call("transport.play", json!({})).unwrap();
        let s = wait(&mut b, |s| s["position"].as_i64().unwrap() >= 4_800);
        assert_eq!(s["playing"], true, "{s}");
        assert!(s["position"].as_i64().unwrap() >= 4_800, "the clock moves the position: {s}");

        // Locate while playing jumps the player; the position keeps counting from there.
        b.call("transport.locate", json!({"at": 480_000})).unwrap();
        let s = wait(&mut b, |s| s["position"].as_i64().unwrap() > 480_000);
        assert!((480_000..528_000).contains(&s["position"].as_i64().unwrap()), "{s}");

        b.call("transport.stop", json!({})).unwrap();
        let stopped = b.status();
        assert_eq!(stopped["playing"], false);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(b.status()["position"], stopped["position"], "a stopped transport stays put");

        // Locate while stopped moves the stopped playhead, and upstream's command agrees.
        b.call("transport.locate", json!({"at": {"seconds": 2.0}})).unwrap();
        assert_eq!(b.status()["position"], 96_000);
        let engine_view = b.call("engine.execute", json!({"command": "transport.status"})).unwrap();
        assert_eq!(engine_view["position"], 96_000);
    }

    #[test]
    fn a_selection_plays_to_its_end_and_stops() {
        let mut b = bridge();
        b.engine.session_mut().edit.selection = Range { start: 0, end: 2_400 };
        b.call("transport.play_selection", json!({})).unwrap();
        let s = wait(&mut b, |s| s["playing"] == false);
        assert_eq!(s["playing"], false, "{s}");
        assert!(s["position"].as_i64().unwrap() >= 2_400, "{s}");
    }

    #[test]
    fn a_new_session_stops_playback_and_record_is_refused() {
        let mut b = bridge();
        b.call("transport.play", json!({"from": 48_000})).unwrap();
        assert_eq!(b.status()["playing"], true);
        b.call("engine.execute", json!({"command": "session.new", "params": {}})).unwrap();
        let s = b.status();
        assert_eq!((s["playing"].as_bool(), s["position"].as_i64()), (Some(false), Some(0)), "{s}");
        assert!(!b.player().is_playing());
        let e = b.call("transport.record", json!({})).unwrap_err();
        assert!(e.contains("does not record"), "{e}");
    }
}
