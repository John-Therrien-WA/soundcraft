//! `soundcraft-bridge`: the control protocol over stdio, for the band's DAW front end.
//!
//! ```text
//! soundcraft-bridge [--control PORT]
//!   --control PORT   also serve the protocol on 127.0.0.1:PORT (0 picks a free port,
//!                    announced on stderr as SOUNDCRAFT_CONTROL_PORT=<port>)
//!   --help, --version
//! stdin: one JSON request per line; stdout: one JSON reply per line; stderr: the log
//! (level from SOUNDCRAFT_BRIDGE_LOG: error|warn|info|debug|trace|off).
//! Exit: 0 stdin closed or bridge.quit; 1 stdin or stdout failed; 2 bad arguments;
//! 4 the control port could not be bound.
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use soundcraft_bridge::{Bridge, logger, serve};
use soundcraft_engine::Engine;
use std::io::Write;
use std::process::ExitCode;

const USAGE: u8 = 2;
const CONTROL_PORT: u8 = 4;

/// The command line, checked up front: a misspelled flag fails before anything opens.
#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    control: Option<u16>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--control" => {
                let v = it.next().ok_or("--control needs a port")?;
                a.control = Some(v.parse().map_err(|_| format!("--control: `{v}` is not a port"))?);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(a)
}

fn help_text() -> String {
    include_str!("main.rs")
        .lines()
        .skip(3)
        .take_while(|l| l.trim_end() != "//! ```")
        .map(|l| l.trim_start_matches("//! "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

fn fail(code: u8, msg: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "soundcraft-bridge: {msg}");
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        say(&help_text());
        return ExitCode::SUCCESS;
    }
    if raw.iter().any(|a| a == "--version" || a == "-V") {
        say(&format!("soundcraft-bridge {}", env!("CARGO_PKG_VERSION")));
        return ExitCode::SUCCESS;
    }
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => return fail(USAGE, &format!("{e} (see --help)")),
    };
    logger::init(std::env::var("SOUNDCRAFT_BRIDGE_LOG").ok().as_deref());
    let bridge = Bridge::new(Engine::default());
    let code = match serve::run(bridge, args.control) {
        Ok(serve::End::Done) => ExitCode::SUCCESS,
        Ok(serve::End::Io(e)) => fail(1, &e),
        Err(e) => fail(CONTROL_PORT, &e),
    };
    // The engine and its plugin instances are gone by now.
    soundcraft_engine::shutdown_plugin_hosts();
    code
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn flags_are_checked_up_front() {
        assert_eq!(parse(&args(&[])).unwrap(), Args::default());
        assert_eq!(parse(&args(&["--control", "0"])).unwrap().control, Some(0));
        assert!(parse(&args(&["--control"])).is_err());
        assert!(parse(&args(&["--control", "x"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
