//! A bridge child process driven over stdio, and fixtures, shared by the integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

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
    /// Everything the child logged on stderr, read by a thread so a chatty log cannot fill the pipe.
    log: Arc<Mutex<Vec<u8>>>,
    drain: Option<std::thread::JoinHandle<()>>,
}

impl Proc {
    pub fn spawn(args: &[&str]) -> Proc {
        let mut child = bin().args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (log, drain) = drain_stderr(child.stderr.take().unwrap());
        Proc { child, stdin, stdout, next: 1, log, drain: Some(drain) }
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
        let status = self.child.wait().unwrap();
        self.stderr_after_exit(status)
    }

    /// The exit status once the child has ended, with all of its stderr.
    pub fn stderr_after_exit(mut self, status: std::process::ExitStatus) -> (std::process::ExitStatus, String) {
        if let Some(d) = self.drain.take() {
            d.join().unwrap();
        }
        (status, String::from_utf8_lossy(&self.log.lock().unwrap()).into_owned())
    }

    /// The control port the child announced on stderr (spawned with `--control 0`).
    pub fn port(&self) -> u16 {
        let t0 = std::time::Instant::now();
        loop {
            let log = String::from_utf8_lossy(&self.log.lock().unwrap()).into_owned();
            if let Some(p) = log.lines().find_map(|l| l.trim().strip_prefix("SOUNDCRAFT_CONTROL_PORT=")) {
                return p.parse().unwrap();
            }
            assert!(t0.elapsed() < std::time::Duration::from_secs(10), "no port announced: {log}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Write raw bytes to the child's stdin (a line the protocol must refuse, say).
    pub fn send_raw(&mut self, bytes: &[u8]) {
        let i = self.stdin.as_mut().unwrap();
        i.write_all(bytes).unwrap();
        i.flush().unwrap();
    }

    /// Read one reply line, whatever its id.
    pub fn read_reply(&mut self) -> Value {
        let mut reply = String::new();
        self.stdout.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("not JSON ({e}): {reply:?}"))
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

/// Collect a child's stderr on a thread until it closes.
pub fn drain_stderr(err: impl std::io::Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let handle = std::thread::spawn(move || {
        let mut err = err;
        let mut chunk = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut err, &mut chunk) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });
    (log, handle)
}

/// A TCP client of the control port.
pub struct Tcp {
    out: std::net::TcpStream,
    input: BufReader<std::net::TcpStream>,
}

impl Tcp {
    pub fn connect(port: u16) -> Tcp {
        let out = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        out.set_read_timeout(Some(std::time::Duration::from_secs(30))).unwrap();
        let input = BufReader::new(out.try_clone().unwrap());
        Tcp { out, input }
    }

    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.out.write_all(bytes).unwrap();
        self.out.flush().unwrap();
    }

    /// One reply line; `None` when the server closed the connection.
    pub fn try_reply(&mut self) -> Option<Value> {
        let mut line = String::new();
        if self.input.read_line(&mut line).unwrap() == 0 {
            return None;
        }
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line:?}")))
    }

    pub fn reply(&mut self) -> Value {
        self.try_reply().expect("the connection was closed")
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.send_raw(format!("{}\n", json!({"id": "t", "method": method, "params": params})).as_bytes());
        self.reply()
    }
}

/// A request line longer than the bridge accepts, newline included.
pub fn oversized_line() -> Vec<u8> {
    let mut v = vec![b'x'; soundcraft_bridge::wire::MAX_LINE + 4096];
    v.push(b'\n');
    v
}
