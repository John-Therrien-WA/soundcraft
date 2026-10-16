//! Our plugins (`wild-animal-plugins`) over stdio: listed beside upstream's, inserted by id,
//! and heard in a bounce.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Proc, sine_wav, tmpdir};

/// Peak of a bounce in `[from, to)` seconds, channel 0.
fn peak_in(path: &std::path::Path, from: f64, to: f64) -> f32 {
    let (_, buf) = soundcraft_audio_io::decode(&std::fs::read(path).unwrap(), Some("wav")).unwrap();
    let ch = &buf.channels[0];
    let (a, b) = ((from * 48_000.0) as usize, ((to * 48_000.0) as usize).min(ch.len()));
    ch[a..b].iter().fold(0.0f32, |m, s| m.max(s.abs()))
}
use serde_json::json;

#[test]
fn wa_plugins_are_listed_inserted_and_rendered() {
    let mut b = Proc::spawn(&["--no-audio"]);
    let list = b.execute("engine.plugins", json!({}));
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    for id in ["wa_eq7", "wa_compressor", "eq_7band", "compressor"] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }

    let dir = tmpdir("wa-plugins");
    let tone = dir.join("Tone.wav");
    sine_wav(&tone, 0.5, 1.0);
    b.execute("session.new", json!({"name": "WA", "sample_rate": 48000}));
    b.execute("file.import_audio", json!({"path": tone.to_str().unwrap()}));
    let track = b.ok("session.inspect", json!({"detail": "summary"}))["tracks"][0]["id"].clone();
    let bounce = |b: &mut Proc, name: &str| {
        let out = dir.join(name);
        b.execute("file.bounce_mix", json!({"path": out.to_str().unwrap(), "start": 0, "end": 48000, "bit_depth": "32f", "dither": false}));
        (peak_in(&out, 0.0, 0.002), peak_in(&out, 0.3, 1.0))
    };
    let (_, dry) = bounce(&mut b, "dry.wav");
    b.execute("mix.insert", json!({"track": track, "slot": 0, "plugin": "wa_eq7", "params": {"output_gain": -6.0}}));
    b.execute("mix.insert", json!({"track": track, "slot": 1, "plugin": "wa_compressor", "params": {"ratio": 1.0}}));
    // The host pushes the insert's parameters on the first block and the plugin glides to them
    // from its defaults (a 20 ms time constant), so the level is read after 300 ms.
    let (head, wet) = bounce(&mut b, "wet.wav");
    println!("first 2 ms {:.3} dB, after 300 ms {:.3} dB", 20.0 * (head / dry).log10(), 20.0 * (wet / dry).log10());
    let db = 20.0 * (wet / dry).log10();
    assert!((db + 6.0).abs() < 0.05, "WA EQ 7 output -6 dB moved the bounce by {db:.3} dB");

    let (status, stderr) = b.finish();
    assert!(status.success(), "{status:?} {stderr}");
    let _ = std::fs::remove_dir_all(dir);
}
