//! The serving loop. The bridge (its engine and audio player) lives on the main thread; stdin and
//! each TCP connection are read on their own threads and their lines handed over in arrival order.

use crate::{Bridge, wire};
use serde_json::json;
use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// TCP connections served at once; further ones get an error line and are closed.
const MAX_CONNECTIONS: usize = 16;
/// How often the bridge looks at the transport when no request arrives.
const TICK: Duration = Duration::from_millis(10);

/// Where a reply goes.
enum Sink {
    Stdout,
    Tcp(Sender<String>),
}

enum Msg {
    Line(String, Sink),
    /// A stdin line that was not a request: the error reply to write.
    Reject(String),
    /// stdin reached its end (or failed): the spawning process is gone or done.
    Closed(Option<String>),
}

/// Why serving ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// stdin closed, or `bridge.quit`.
    Done,
    /// stdout could not be written, or stdin failed.
    Io(String),
}

/// Bind the control port (`0` picks a free one) and serve it in the background. The port is
/// announced on stderr as `SOUNDCRAFT_CONTROL_PORT=<port>`, as the app does on stdout.
fn listen(port: u16, tx: Sender<Msg>) -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let actual = listener.local_addr().map_err(|e| e.to_string())?.port();
    let _ = writeln!(std::io::stderr(), "SOUNDCRAFT_CONTROL_PORT={actual}");
    log::info!("control channel on 127.0.0.1:{actual}");
    std::thread::Builder::new().name("bridge-control".into()).spawn(move || accept_loop(listener, tx)).map_err(|e| e.to_string())?;
    Ok(actual)
}

struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, tx: Sender<Msg>) {
    let open = Arc::new(AtomicUsize::new(0));
    for mut stream in listener.incoming().flatten() {
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "too many control connections"}));
            continue;
        }
        let slot = Slot(Arc::clone(&open));
        let tx = tx.clone();
        let _ = std::thread::Builder::new().name("bridge-control-conn".into()).spawn(move || {
            let _slot = slot;
            serve_tcp(stream, tx);
        });
    }
}

fn serve_tcp(stream: TcpStream, tx: Sender<Msg>) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        let line = match wire::next_request(&mut reader, &mut buf, wire::MAX_LINE) {
            wire::Next::Line(line) => line,
            wire::Next::Reject(reply) => {
                if writeln!(out, "{reply}").is_err() {
                    break;
                }
                continue;
            }
            wire::Next::Eof | wire::Next::Failed(_) => break,
        };
        let (rtx, rrx) = channel();
        if tx.send(Msg::Line(line, Sink::Tcp(rtx))).is_err() {
            break;
        }
        let Ok(reply) = rrx.recv() else { break };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

fn read_stdin(tx: Sender<Msg>) {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut buf = Vec::new();
    loop {
        let why = match wire::next_request(&mut reader, &mut buf, wire::MAX_LINE) {
            wire::Next::Line(line) => {
                if tx.send(Msg::Line(line, Sink::Stdout)).is_err() {
                    return;
                }
                continue;
            }
            wire::Next::Reject(reply) => {
                if tx.send(Msg::Reject(reply)).is_err() {
                    return;
                }
                continue;
            }
            wire::Next::Eof => None,
            wire::Next::Failed(e) => Some(e),
        };
        let _ = tx.send(Msg::Closed(why));
        return;
    }
}

/// Serve stdin (and the control port when given) until stdin closes or `bridge.quit`.
pub fn run(mut bridge: Bridge, control: Option<u16>) -> Result<End, String> {
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = channel();
    if let Some(port) = control {
        listen(port, tx.clone())?;
    }
    std::thread::Builder::new().name("bridge-stdin".into()).spawn(move || read_stdin(tx)).map_err(|e| e.to_string())?;
    let stdout = std::io::stdout();
    let end = loop {
        match rx.recv_timeout(TICK) {
            Ok(Msg::Line(line, sink)) => {
                let reply = wire::answer(&mut bridge, &line);
                match sink {
                    Sink::Stdout => {
                        let mut o = stdout.lock();
                        if let Err(e) = writeln!(o, "{reply}").and_then(|()| o.flush()) {
                            break End::Io(format!("stdout: {e}"));
                        }
                    }
                    Sink::Tcp(r) => {
                        let _ = r.send(reply);
                    }
                }
                if bridge.quit_requested() {
                    // Let a TCP connection thread write the quit reply before the process ends.
                    std::thread::sleep(Duration::from_millis(50));
                    break End::Done;
                }
            }
            Ok(Msg::Reject(reply)) => {
                let mut o = stdout.lock();
                if let Err(e) = writeln!(o, "{reply}").and_then(|()| o.flush()) {
                    break End::Io(format!("stdout: {e}"));
                }
            }
            Ok(Msg::Closed(None)) => break End::Done,
            Ok(Msg::Closed(Some(e))) => break End::Io(format!("stdin: {e}")),
            Err(RecvTimeoutError::Timeout) => bridge.tick(),
            Err(RecvTimeoutError::Disconnected) => break End::Done,
        }
    };
    drop(bridge);
    Ok(end)
}
