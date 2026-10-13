//! Device and buffer choice, run as the real binary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Proc;
use serde_json::json;

#[test]
fn the_output_devices_are_listed() {
    let mut b = Proc::spawn(&["--no-audio"]);
    let d = b.ok("devices", json!({}));
    let outs = d["outputs"].as_array().unwrap();
    for o in outs {
        assert!(o["name"].is_string() && o["buffer_sizes"].is_array(), "{o}");
    }
    assert!(outs.iter().filter(|o| o["is_default"] == true).count() <= 1, "{d}");
    assert!(b.finish().0.success());
}

#[test]
fn an_unknown_device_is_refused_with_the_list() {
    let out = common::bin().args(["--device", "No Such Device 1234"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown output device `No Such Device 1234`; the outputs are:"), "{err}");
    assert!(out.stdout.is_empty(), "nothing on the protocol channel");
}

#[test]
fn an_impossible_buffer_is_refused() {
    let out = common::bin().args(["--buffer", "4"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("buffer 4"));
}

#[test]
#[ignore = "opens the default output device; CI machines have none"]
fn the_requested_buffer_is_the_one_reported() {
    let mut b = Proc::spawn(&["--buffer", "256"]);
    let s = b.ok("transport.status", json!({}));
    assert_eq!(s["device"]["buffer"], 256, "{s}");
    assert_eq!(s["device"]["silent"], false);
    assert!(b.finish().0.success());
}
