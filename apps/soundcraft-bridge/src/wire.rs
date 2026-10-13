//! The wire format: one JSON request per line in, one JSON reply per line out
//! (`{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`), as upstream's control
//! channel has it (`docs/control-protocol.md`).

use crate::Bridge;
use serde_json::{Value, json};
use std::io::{BufRead, Read};

/// Longest accepted request line (bytes, without the newline): the app's bound, room for the
/// largest plugin state `mix.insert_state` carries plus 1 MiB of JSON around it.
pub const MAX_LINE: usize = soundcraft_engine::model::b64::MAX_DECODED_BYTES.div_ceil(3) * 4 + (1 << 20);

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline and a trailing `\r`
/// stripped). `Ok(false)` at end of stream; `Err` when the line is too long or the read fails.
pub fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, String> {
    buf.clear();
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = reader.take(limit).read_until(b'\n', buf).map_err(|e| e.to_string())?;
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
        return Err(format!("request line longer than {max} bytes"));
    }
    Ok(true)
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
        assert!(read_bounded_line(&mut input, &mut buf, 10).is_err());
    }

    #[test]
    fn replies_carry_the_id_and_the_outcome() {
        let mut b = Bridge::new(soundcraft_engine::Engine::default());
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
