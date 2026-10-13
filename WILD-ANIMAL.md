# soundcraft, Wild Animal's fork: the DAW's native audio backend

This checkout is Wild Animal's fork of `storytold/soundcraft`, the agent-built Pro Tools clone in Rust (MIT or Apache 2.0, first commit 6 October 2026). Upstream's own agent entry point is `AGENTS.md` and stays theirs. This file is ours: the band's DAW has its front end in `../Trickfilm` (`pipeline/lab/`, the timeline and playback as John sees them), its analysis and offline DSP as tools of `../Consonance`'s `daw/` engine, and here the native pieces a browser cannot give: a real-time CoreAudio engine, Audio Unit, VST3 and CLAP hosting, MIDI device I/O, recording, an offline mix engine. John's decision of 12 and 13 October 2026 (`../robot-release/sessions/2026-10-13/session.md`): "we can take the hard backend stuff from soundcraft as it develops, and have our UI more logic flavored."

## How the fork is kept

- `upstream` is `https://github.com/storytold/soundcraft`; `main` tracks `upstream/main` and never carries our commits. `origin` is John's private repo `git@github-personal:John-Therrien-WA/soundcraft.git`; nothing is published from here.
- **Our delta lives on `wild-animal` and stays small:** the bridge crate below, this file, one import line in `CLAUDE.md`, and nothing inside upstream's crates unless the same fix goes upstream as a pull request and is carried here only until it lands. Taking upstream's progress is `git fetch upstream`, a rebase of `wild-animal` onto `upstream/main`, then the checks. A conflict inside upstream's crates means our delta grew where it should not.
- The toolchain is `~/.cargo/bin` (rustup, cargo 1.99; not on PATH by default). A build writes `target/` here (6.6 GB in debug) and the registry in `~/.cargo`, which the sandbox refuses: builds run hatched, John approving the prompt.
- Inside a sandboxed shell the Audio Unit registry is empty and the VST3 scan crashes (SIGTRAP); plugin work runs hatched. Logic's own instruments are not system Audio Units; nothing but Logic hosts them.

## What upstream gives, as established on 12 October 2026 (0.6.1, commit 6cf896e)

From a read of the code and a run of the whole suite on John's Mac (about 690 tests, all passing, `au-host/tests/system_units.rs` driving Apple's shipped units):

- **Worth taking as they are:** `clap-host`, `vst3-host`, `au-host` (own FFI, in-process, each depending only on `dsp`), `mix` (offline and real-time render with plugin delay compensation), `model` (`Session`, `.scraft` JSON, undo snapshots, automation lanes), `time`, `audio-io`, `midi` (SMF, 960 PPQ), the capture ring in `playback/record.rs`, `engine` (about 469 JSON commands behind one registry), `automation` (MCP).
- **Missing or weak, confirmed in code:** no headless real-time audio (sound plays only through the egui app; the CLI lacks `playback`); no device or buffer-size choice (cpal's default device, a fixed 512 block); no MIDI device input; no recording latency compensation; plugins get no tempo or transport (VST3 hard-coded to 120 BPM in 4/4, CLAP null); all hosting in-process (a bad plugin kills the process); the audio callback hands work to the global rayon pool and allocates, so dropouts under load are expected until that changes, upstream or here.
- **Measured here:** a two-track bounce in 0.06 s with gain and pan to 0.02 dB; Apple's AUDelay inserted and a parameter set by command; 85 AUs, 59 VST3 and 3 CLAP found outside the sandbox; the CLI starts in about 6 ms; the MCP process answers in under a millisecond.

## Our crate: `apps/soundcraft-bridge`

The headless binary the lab talks to. It links `engine`, `playback` and the three hosts, carries the transport glue the egui app holds in one screen (`ui-egui/src/lib.rs`, where engine transport requests become `Player` calls and the recorder is opened), and speaks upstream's newline-delimited JSON control protocol (`docs/control-protocol.md`: `engine.execute`, `session.inspect`, the transport methods) over stdio, with `--control <port>` for TCP as the app has. It adds what the lab needs and upstream lacks, in the order the lab reaches for each: the choice of output device and buffer size, a transport that reports its position by sample, plugin tempo and transport from the session. Each addition ships with a test in the crate.

The lab finds the binary at `../soundcraft/target/release/soundcraft-bridge` (or `SOUNDCRAFT_BRIDGE`), spawns it and talks over stdio.

## Rules

- Living docs, git is the history; no em dashes. Fail loud. Nothing of John's is read or written by this repo except through the lab.
- `target/` stays ignored as upstream has it.
