//! The control port, run as the real binary: concurrent clients, lines the protocol must refuse
//! without dropping the connection, `bridge.quit`, and the connection cap.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Proc, Tcp, oversized_line};
use serde_json::json;

fn spawn() -> (Proc, u16) {
    let b = Proc::spawn(&["--no-audio", "--control", "0"]);
    let port = b.port();
    (b, port)
}

#[test]
fn two_clients_are_served_at_once() {
    let (b, port) = spawn();
    let workers: Vec<_> = (0..2)
        .map(|n| {
            std::thread::spawn(move || {
                let mut c = Tcp::connect(port);
                for i in 0..50 {
                    let r = c.request("transport.status", json!({}));
                    assert_eq!(r["ok"], true, "client {n} request {i}: {r}");
                }
            })
        })
        .collect();
    // A third client and the stdio channel are served meanwhile.
    let mut c = Tcp::connect(port);
    assert_eq!(c.request("session.inspect", json!({}))["ok"], true);
    for w in workers {
        w.join().unwrap();
    }
    let (status, stderr) = b.finish();
    assert!(status.success(), "{stderr}");
}

#[test]
fn lines_that_are_not_requests_are_answered_and_the_connection_goes_on() {
    let (b, port) = spawn();
    let mut c = Tcp::connect(port);

    c.send_raw(b"{not json\n");
    let r = c.reply();
    assert_eq!((r["id"].is_null(), r["ok"].as_bool()), (true, Some(false)), "{r}");
    assert!(r["error"].as_str().unwrap().starts_with("bad JSON"), "{r}");

    c.send_raw(b"{\"id\": 5}\n");
    let r = c.reply();
    assert_eq!((r["id"].as_i64(), r["ok"].as_bool()), (Some(5), Some(false)), "{r}");
    assert!(r["error"].as_str().unwrap().contains("`method` required"), "{r}");

    c.send_raw(b"\xff\xfe not utf-8\n");
    let r = c.reply();
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("UTF-8"), "{r}");

    c.send_raw(&oversized_line());
    let r = c.reply();
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("longer than"), "{r}");

    // The reader found its way to the next line: the connection still answers.
    assert_eq!(c.request("transport.status", json!({}))["ok"], true);
    // And the bridge as a whole is still up for other clients.
    assert_eq!(Tcp::connect(port).request("transport.status", json!({}))["ok"], true);
    let (status, stderr) = b.finish();
    assert!(status.success(), "{stderr}");
}

#[test]
fn bridge_quit_over_tcp_replies_and_ends_the_bridge() {
    let (mut b, port) = spawn();
    let mut c = Tcp::connect(port);
    let r = c.request("bridge.quit", json!({}));
    assert_eq!((r["ok"].as_bool(), &r["result"]), (Some(true), &json!({})), "{r}");
    let status = b.child.wait().unwrap();
    assert!(status.success(), "{status:?}");
}

#[test]
fn connections_beyond_the_cap_are_told_so() {
    let (b, port) = spawn();
    let mut held: Vec<Tcp> = (0..16).map(|_| Tcp::connect(port)).collect();
    for c in &mut held {
        assert_eq!(c.request("transport.status", json!({}))["ok"], true);
    }
    let mut extra = Tcp::connect(port);
    let r = extra.reply();
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("too many"), "{r}");
    assert!(extra.try_reply().is_none(), "the extra connection is closed");
    drop(held);
    let (status, stderr) = b.finish();
    assert!(status.success(), "{stderr}");
}
