//! The protocol over stdio, run as the real binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Proc, peak, sine_wav, tmpdir};
use serde_json::json;

#[test]
fn a_session_is_built_mixed_and_bounced_over_stdio() {
    let dir = tmpdir("bounce");
    let tone = dir.join("Tone.wav");
    sine_wav(&tone, 0.5, 1.0);
    let mut b = Proc::spawn(&[]);

    let s = b.execute("session.new", json!({"name": "Bridge", "sample_rate": 48000}));
    assert_eq!(s["sample_rate"], 48000, "{s}");
    b.execute("file.import_audio", json!({"path": tone.to_str().unwrap()}));
    let doc = b.ok("session.inspect", json!({"detail": "summary"}));
    let track = doc["tracks"][0]["id"].clone();
    assert!(track.is_number(), "{doc}");

    let bounce = |b: &mut Proc, name: &str| {
        let out = dir.join(name);
        b.execute("file.bounce_mix", json!({"path": out.to_str().unwrap(), "start": 0, "end": 48000, "bit_depth": "32f", "dither": false}));
        peak(&out)
    };
    let unity = bounce(&mut b, "unity.wav");
    assert!(unity > 0.1, "the bounce carries the tone: peak {unity}");
    b.execute("mix.volume", json!({"track": track, "db": -6.0}));
    let down = bounce(&mut b, "down.wav");
    let db = 20.0 * (down / unity).log10();
    assert!((db + 6.0).abs() < 0.02, "mix.volume -6 dB moved the bounce by {db:.3} dB");

    // Errors keep the reply shape and the process alive.
    let bad = b.request("engine.execute", json!({"command": "mix.volume", "params": {"track": "Nope", "db": -6}}));
    assert_eq!(bad["ok"], false);
    assert!(bad["error"].as_str().unwrap().contains("Nope"), "{bad}");
    assert_eq!(b.request("ui.click", json!({}))["ok"], false);
    assert_eq!(b.request("no.such.method", json!({}))["ok"], false);

    let (status, stderr) = b.finish();
    assert!(status.success(), "closing stdin ends the bridge cleanly: {status:?} {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bridge_quit_ends_the_process_after_its_reply() {
    let mut b = Proc::spawn(&[]);
    assert_eq!(b.ok("bridge.quit", json!({})), json!({}));
    let status = b.child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn bad_arguments_exit_with_2() {
    let out = common::bin().arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bogus"));
}

#[test]
fn the_control_port_speaks_the_same_protocol() {
    use std::io::{BufRead, BufReader, Write};
    let mut child = common::bin()
        .args(["--control", "0"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut err = BufReader::new(child.stderr.take().unwrap());
    let port = loop {
        let mut line = String::new();
        assert!(err.read_line(&mut line).unwrap() > 0, "no port announced");
        if let Some(p) = line.trim().strip_prefix("SOUNDCRAFT_CONTROL_PORT=") {
            break p.parse::<u16>().unwrap();
        }
    };
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    writeln!(c, r#"{{"id":"t1","method":"engine.execute","params":{{"command":"track.new","params":{{"count":2,"format":"stereo"}}}}}}"#).unwrap();
    writeln!(c, r#"{{"id":"t2","method":"session.inspect","params":{{}}}}"#).unwrap();
    let mut r = BufReader::new(c);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!((v["id"].as_str(), v["ok"].as_bool()), (Some("t1"), Some(true)), "{line}");
    line.clear();
    r.read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["result"]["tracks"].as_array().map(Vec::len), Some(2), "{line}");
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}
