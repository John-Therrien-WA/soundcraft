//! Log records go to stderr, one line each; stdout carries only the protocol.

use std::io::Write;

struct Stderr;

static LOGGER: Stderr = Stderr;

impl log::Log for Stderr {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let _ = writeln!(std::io::stderr().lock(), "[{} {}] {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {}
}

/// Install the stderr logger. `level` is `error`, `warn`, `info` (the default), `debug`, `trace` or
/// `off`, from `SOUNDCRAFT_BRIDGE_LOG`.
pub fn init(level: Option<&str>) {
    let filter = level.and_then(|l| l.parse::<log::LevelFilter>().ok()).unwrap_or(log::LevelFilter::Info);
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(filter);
    }
}
