//! A bridge child process driven over stdio, and fixtures, shared by the integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soundcraft-bridge"));
    cmd.env("SOUNDCRAFT_NO_PREFS", "1");
    cmd
}

pub struct Proc {
    pub child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Proc {
    pub fn spawn(args: &[&str]) -> Proc {
        let mut child = bin().args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Proc { child, stdin, stdout, next: 1 }
    }

    /// Send one request; return the whole reply object (its `id` checked).
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        writeln!(self.stdin.as_mut().unwrap(), "{line}").unwrap();
        let mut reply = String::new();
        self.stdout.read_line(&mut reply).unwrap();
        let v: Value = serde_json::from_str(&reply).unwrap_or_else(|e| panic!("not JSON ({e}): {reply:?}"));
        assert_eq!(v["id"], json!(id), "{v}");
        v
    }

    /// Send one request that must succeed; return its result.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.request(method, params);
        assert_eq!(v["ok"], true, "{method}: {v}");
        v["result"].clone()
    }

    pub fn execute(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    /// Close stdin and wait: the exit status and stderr.
    pub fn finish(mut self) -> (std::process::ExitStatus, String) {
        drop(self.stdin.take());
        let out = self.child.wait_with_output().unwrap();
        (out.status, String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

pub fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sc-bridge-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A mono 32-bit float WAV of a 1 kHz sine at `amp`, `secs` long, at 48 kHz.
pub fn sine_wav(path: &Path, amp: f32, secs: f32) {
    let rate = 48_000u32;
    let n = (secs * rate as f32) as usize;
    let samples = (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin()).collect();
    let buf = soundcraft_audio_io::AudioBuffer { sample_rate: rate, channels: vec![samples] };
    let opts = soundcraft_audio_io::EncodeOptions { bit_depth: soundcraft_audio_io::BitDepth::Float32, ..Default::default() };
    std::fs::write(path, soundcraft_audio_io::encode(&buf, &opts).unwrap()).unwrap();
}

/// Peak of every channel of a decoded file.
pub fn peak(path: &Path) -> f32 {
    let bytes = std::fs::read(path).unwrap();
    let (_, buf) = soundcraft_audio_io::decode(&bytes, Some("wav")).unwrap();
    buf.channels.iter().flatten().fold(0.0f32, |m, s| m.max(s.abs()))
}
