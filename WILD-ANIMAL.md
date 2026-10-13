# soundcraft, Wild Animal's fork: the DAW's native audio backend

This checkout is Wild Animal's fork of `storytold/soundcraft`, the agent-built Pro Tools clone in Rust (MIT or Apache 2.0, first commit 6 October 2026). Upstream's own agent entry point is `AGENTS.md` and stays theirs. This file is ours: the band's DAW has its front end in `../Trickfilm` (`pipeline/lab/`, the timeline and playback as John sees them), its analysis and offline DSP as tools of `../Consonance`'s `daw/` engine, and here the native pieces a browser cannot give: a real-time CoreAudio engine, Audio Unit, VST3 and CLAP hosting, MIDI device I/O, recording, an offline mix engine. John's decision of 12 and 13 October 2026 (`../robot-release/sessions/2026-10-13/session.md`): "we can take the hard backend stuff from soundcraft as it develops, and have our UI more logic flavored."

## How the fork is kept

- `upstream` is `https://github.com/storytold/soundcraft`; `main` tracks `upstream/main` and never carries our commits. `origin` is John's private repo `git@github-personal:John-Therrien-WA/soundcraft.git`; nothing is published from here.
- **Our delta lives on `wild-animal` and stays small:** the bridge crate below (and its entry in `Cargo.lock`), this file, one import line in `CLAUDE.md`, and nothing inside upstream's crates unless the same fix goes upstream as a pull request and is carried here only until it lands. Taking upstream's progress is `git fetch upstream`, a rebase of `wild-animal` onto `upstream/main`, then the checks. A conflict inside upstream's crates means our delta grew where it should not.
- The toolchain is `~/.cargo/bin` (rustup, cargo 1.99; not on PATH by default). A build writes `target/` here (6.6 GB in debug) and the registry in `~/.cargo`, which the sandbox refuses: builds run hatched, John approving the prompt.
- Inside a sandboxed shell the Audio Unit registry is empty and the VST3 scan crashes (SIGTRAP); plugin work runs hatched. Logic's own instruments are not system Audio Units; nothing but Logic hosts them.

## What upstream gives, as established on 12 October 2026 (0.6.1, commit 6cf896e)

From a read of the code and a run of the whole suite on John's Mac (about 690 tests, all passing, `au-host/tests/system_units.rs` driving Apple's shipped units):

- **Worth taking as they are:** `clap-host`, `vst3-host`, `au-host` (own FFI, in-process, each depending only on `dsp`), `mix` (offline and real-time render with plugin delay compensation), `model` (`Session`, `.scraft` JSON, undo snapshots, automation lanes), `time`, `audio-io`, `midi` (SMF, 960 PPQ), the capture ring in `playback/record.rs`, `engine` (about 469 JSON commands behind one registry), `automation` (MCP).
- **Missing or weak, confirmed in code:** no headless real-time audio (sound plays only through the egui app; the CLI lacks `playback`); no device or buffer-size choice (cpal's default device, a fixed 512 block); no MIDI device input; no recording latency compensation; plugins get no tempo or transport (VST3 hard-coded to 120 BPM in 4/4, CLAP null); all hosting in-process (a bad plugin kills the process); the audio callback hands work to the global rayon pool and allocates, so dropouts under load are expected until that changes, upstream or here.
- **Measured here:** a two-track bounce in 0.06 s with gain and pan to 0.02 dB; Apple's AUDelay inserted and a parameter set by command; 85 AUs, 59 VST3 and 3 CLAP found outside the sandbox; the CLI starts in about 6 ms; the MCP process answers in under a millisecond.

## Our crate: `apps/soundcraft-bridge`

The headless binary the lab talks to. It runs upstream's `engine` (and through it the three plugin hosts) without a window, plays through `playback::Player` on an output device, and speaks upstream's newline-delimited JSON control protocol (`docs/control-protocol.md`) over stdio, and over `127.0.0.1` with `--control <port>` as the app has it. stdout carries only the protocol; the log goes to stderr. Its `README.md` is the protocol reference for the lab's builder: every method with its params and result, the flags, the exit codes, how to spawn it.

- **Methods:** `engine.execute {command, params}` (every engine command), `engine.commands`, `engine.parity`, `session.inspect {detail}`, the `transport.*` commands as methods of their own, `transport.status` (playing, the position in session samples, the device's actual rate, buffer and channels, any output trouble), `devices` (the output devices with their rates and buffer ranges), `bridge.quit`.
- **Flags:** `--device <name>` and `--buffer <frames>`, checked against the device list before anything opens (an unknown device is refused with the list, a buffer outside the device's range is refused, a device that does not take the buffer is refused); `--no-audio` (upstream's silent clock); `--control <port>`. The bridge never falls back to silence on its own: exit 3.
- **The transport glue** is the app's (`ui-egui/src/lib.rs`, `handle_transport`, `start_play`, `stop_play`) without the UI: after every command the engine's transport requests become `Player` calls, and while playing the player's position is read back into `engine.transport`. The dispatch is the bridge's own: the app's control server needs egui, and `automation::Headless` drops transport requests.
- **Tests:** `~/.cargo/bin/cargo test -p soundcraft-bridge` (hatched). A child process is driven over stdio through `session.new`, `file.import_audio`, `mix.volume` and `file.bounce_mix`, and two bounces 6 dB apart are checked; the transport runs on the silent clock in process and over stdio; device and buffer choice is checked against synthetic and real device lists. Two tests open the default output and are ignored by default (`-- --ignored` runs them, hatched).
- **The lab** finds the binary at `../soundcraft/target/release/soundcraft-bridge` (or `SOUNDCRAFT_BRIDGE`), built with `~/.cargo/bin/cargo build --release -p soundcraft-bridge`, spawns it with piped stdio, and ends it by closing stdin.

What still needs upstream, or more of the bridge:

- `Player::silent` in `crates/playback` (the silent clock on request, for `--no-audio`) is our one change inside upstream's crates (a constructor and its test), in its own commit, to be filed as a pull request.
- `cargo xtask layers` reports `soundcraft-bridge` as not registered: upstream's check exempts only its own three apps.
- Recording (`transport.record` is refused), MIDI device input, plugin tempo and transport, recording latency compensation, out-of-process plugin hosting, and an audio callback that neither allocates nor hands work to the rayon pool. In the order the lab reaches for them, plugin tempo and transport from the session comes next.
- The position advances in steps of the mix block (512 session samples), and a device running at its own default buffer does not report that size.

## Rules

- Living docs, git is the history; no em dashes. Fail loud. Nothing of John's is read or written by this repo except through the lab.
- `target/` stays ignored as upstream has it.
