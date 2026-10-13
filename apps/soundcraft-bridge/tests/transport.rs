//! The transport over stdio, run as the real binary: on the silent clock, and on the default
//! output device (ignored: it opens the machine's sound output).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Proc, sine_wav, tmpdir};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Play a 2 s session from 0 and read `transport.status` twice while it plays; return both.
fn play_and_read(b: &mut Proc) -> (Value, Value) {
    b.ok("transport.play", json!({"from": 0}));
    std::thread::sleep(Duration::from_millis(400));
    let first = b.ok("transport.status", json!({}));
    std::thread::sleep(Duration::from_millis(600));
    let second = b.ok("transport.status", json!({}));
    (first, second)
}

fn check_positions(first: &Value, second: &Value, t: Duration) {
    assert_eq!((first["playing"].as_bool(), second["playing"].as_bool()), (Some(true), Some(true)), "{first} {second}");
    let (a, b) = (first["position"].as_i64().unwrap(), second["position"].as_i64().unwrap());
    // 600 ms apart at 48 kHz is 28 800 samples; allow scheduling and one device buffer either way.
    let moved = b - a;
    assert!((20_000..40_000).contains(&moved), "moved {moved} samples between reads ({a} to {b}) in {t:?}");
}

#[test]
fn the_transport_runs_on_the_silent_clock() {
    let mut b = Proc::spawn(&["--no-audio"]);
    b.execute("session.new", json!({"sample_rate": 48000}));
    let t0 = Instant::now();
    let (first, second) = play_and_read(&mut b);
    check_positions(&first, &second, t0.elapsed());
    assert_eq!(second["device"]["silent"], true);
    b.ok("transport.stop", json!({}));
    let stopped = b.ok("transport.status", json!({}));
    assert_eq!(stopped["playing"], false);
    b.ok("transport.locate", json!({"at": 12_345}));
    assert_eq!(b.ok("transport.status", json!({}))["position"], 12_345);
    let (status, stderr) = b.finish();
    assert!(status.success(), "{stderr}");
}

#[test]
#[ignore = "opens the default output device and plays a quiet tone; CI machines have none"]
fn the_transport_runs_on_the_default_output() {
    let dir = tmpdir("device");
    let tone = dir.join("Tone.wav");
    sine_wav(&tone, 0.1, 2.0); // -20 dBFS
    let mut b = Proc::spawn(&[]);
    b.execute("session.new", json!({"sample_rate": 48000}));
    b.execute("file.import_audio", json!({"path": tone.to_str().unwrap()}));
    let t0 = Instant::now();
    let (first, second) = play_and_read(&mut b);
    check_positions(&first, &second, t0.elapsed());
    assert_eq!(second["device"]["silent"], false, "{second}");
    assert!(second["device"]["sample_rate"].as_u64().unwrap() > 0);
    b.ok("transport.stop", json!({}));
    let (status, stderr) = b.finish();
    assert!(status.success(), "{stderr}");
    let _ = std::fs::remove_dir_all(dir);
}
