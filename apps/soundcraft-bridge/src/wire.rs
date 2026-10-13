//! The wire format: one JSON request per line in, one JSON reply per line out
//! (`{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`), as upstream's control
//! channel has it (`docs/control-protocol.md`).

use crate::Bridge;
use serde_json::{Value, json};
use std::io::{BufRead, Read};

/// Longest accepted request line (bytes, without the newline): the app's bound, room for the
/// largest plugin state `mix.insert_state` carries plus 1 MiB of JSON around it.
pub const MAX_LINE: usize = soundcraft_engine::model::b64::MAX_DECODED_BYTES.div_ceil(3) * 4 + (1 << 20);

/// What one read of a request stream gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// A request line (never empty or blank).
    Line(String),
    /// A line that cannot be a request (too long, or not UTF-8), already skipped to its newline:
    /// the reply to send, and then reading goes on.
    Reject(String),
    /// End of stream.
    Eof,
    /// The stream failed.
    Failed(String),
}

/// The reply for a request that never became a line.
pub fn rejection(error: &str) -> String {
    json!({"id": null, "ok": false, "error": error}).to_string()
}

/// Read the next request from `reader`: blank lines are skipped, and a line that is too long or
/// not UTF-8 is answered with an error reply rather than ending the stream.
pub fn next_request(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Next {
    loop {
        match read_bounded_line(reader, buf, max) {
            Ok(true) => {}
            Ok(false) => return Next::Eof,
            Err(ReadError::TooLong) => return Next::Reject(rejection(&format!("request line longer than {max} bytes; it was skipped"))),
            Err(ReadError::Io(e)) => return Next::Failed(e),
        }
        match std::str::from_utf8(buf) {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => return Next::Line(line.to_string()),
            Err(_) => return Next::Reject(rejection("request line is not UTF-8; it was skipped")),
        }
    }
}

/// Why a line could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Longer than the limit; the rest of the line has been discarded.
    TooLong,
    Io(String),
}

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline and a trailing `\r`
/// stripped). `Ok(false)` at end of stream. A line over `max` is discarded up to its newline, so
/// the next read starts on the next line, and reported as [`ReadError::TooLong`].
pub fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, ReadError> {
    buf.clear();
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = reader.by_ref().take(limit).read_until(b'\n', buf).map_err(|e| ReadError::Io(e.to_string()))?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(true);
    }
    if buf.len() > max {
        buf.clear();
        skip_line(reader).map_err(|e| ReadError::Io(e.to_string()))?;
        return Err(ReadError::TooLong);
    }
    Ok(true)
}

/// Discard input up to and including the next newline (or the end of the stream).
fn skip_line(reader: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(());
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => {
                reader.consume(i + 1);
                return Ok(());
            }
            None => {
                let n = chunk.len();
                reader.consume(n);
            }
        }
    }
}

/// Answer one request line with one reply line (no newline).
pub fn answer(bridge: &mut Bridge, line: &str) -> String {
    let msg = match serde_json::from_str::<Value>(line) {
        Ok(m) => m,
        Err(e) => return json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}).to_string(),
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return json!({"id": id, "ok": false, "error": "`method` required"}).to_string();
    };
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    match bridge.call(method, params) {
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    }
    .to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_read_up_to_the_limit() {
        let mut buf = Vec::new();
        let mut input: &[u8] = b"aaaaaaaaaa\nbb\r\ncc";
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"aaaaaaaaaa");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"bb");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(true));
        assert_eq!(buf, b"cc");
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(false));
        let long = [b'x'; 100];
        let mut input: &[u8] = &long;
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Err(ReadError::TooLong));
        assert_eq!(read_bounded_line(&mut input, &mut buf, 10), Ok(false));
    }

    #[test]
    fn a_long_line_is_skipped_and_the_next_one_read() {
        let mut input = Vec::new();
        input.extend_from_slice(&[b'x'; 5000]);
        input.extend_from_slice(b"\nok\n\xff\xfe\n   \nlast\n");
        // A small buffer makes the skip cross several fills.
        let mut r = std::io::BufReader::with_capacity(16, &input[..]);
        let mut buf = Vec::new();
        let rejected = |n: Next| matches!(n, Next::Reject(m) if m.contains("\"ok\":false"));
        assert!(rejected(next_request(&mut r, &mut buf, 100)));
        assert_eq!(next_request(&mut r, &mut buf, 100), Next::Line("ok".into()));
        assert!(rejected(next_request(&mut r, &mut buf, 100)), "not UTF-8");
        assert_eq!(next_request(&mut r, &mut buf, 100), Next::Line("last".into()), "blank lines are skipped");
        assert_eq!(next_request(&mut r, &mut buf, 100), Next::Eof);
    }

    #[test]
    fn replies_carry_the_id_and_the_outcome() {
        let mut b = Bridge::silent(soundcraft_engine::Engine::default());
        let ok: Value = serde_json::from_str(&answer(&mut b, r#"{"id":7,"method":"session.inspect"}"#)).unwrap();
        assert_eq!((ok["id"].as_i64(), ok["ok"].as_bool()), (Some(7), Some(true)), "{ok}");
        assert!(ok["result"]["tracks"].is_array());
        let bad: Value = serde_json::from_str(&answer(&mut b, r#"{"id":"x","method":"nope"}"#)).unwrap();
        assert_eq!((bad["id"].as_str(), bad["ok"].as_bool()), (Some("x"), Some(false)));
        assert!(bad["error"].as_str().unwrap().contains("unknown method"));
        let junk: Value = serde_json::from_str(&answer(&mut b, "{not json")).unwrap();
        assert_eq!(junk["ok"], false);
        assert!(junk["error"].as_str().unwrap().starts_with("bad JSON"));
    }
}
