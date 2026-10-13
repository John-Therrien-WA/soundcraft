//! The bridge: an engine answering control-protocol methods without a window.

use serde_json::{Value, json};
use soundcraft_engine::Engine;

/// An engine and what the protocol needs around it.
pub struct Bridge {
    pub engine: Engine,
    quit: bool,
}

impl Bridge {
    pub fn new(engine: Engine) -> Bridge {
        Bridge { engine, quit: false }
    }

    /// True once `bridge.quit` was answered.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Answer one control-protocol method.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str).ok_or("`command` required")?;
                let p = params.get("params").cloned().unwrap_or_else(|| json!({}));
                self.execute(id, &p)
            }
            "engine.commands" => self.engine.execute("engine.commands", &params).map_err(|e| e.to_string()),
            "engine.parity" => Ok(soundcraft_engine::catalog::parity_json()),
            "session.inspect" | "document.inspect" => {
                Ok(soundcraft_engine::inspect::session(&self.engine, params.get("detail").and_then(Value::as_str) == Some("full")))
            }
            "bridge.quit" | "app.quit" => {
                self.quit = true;
                Ok(json!({}))
            }
            m if m.starts_with("ui.") => Err(format!("`{m}` needs the desktop app; the bridge has no window")),
            other => Err(format!("unknown method `{other}`")),
        }
    }

    fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let r = self.engine.execute(id, params).map_err(|e| e.to_string());
        self.engine.transport_requests.clear();
        r
    }
}
