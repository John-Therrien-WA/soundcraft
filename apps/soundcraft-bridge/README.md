# soundcraft-bridge

The headless SoundCraft backend for Wild Animal's DAW front end (the lab in `../Trickfilm/pipeline/lab/`). It runs upstream's engine without a window, plays through a real-time `playback::Player` on an output device, and speaks upstream's control protocol (`docs/control-protocol.md`) over stdio, and over TCP with `--control`.

## Spawning it

The binary is `target/release/soundcraft-bridge` in this checkout (the lab finds it at `../soundcraft/target/release/soundcraft-bridge`, or at `SOUNDCRAFT_BRIDGE`). Build it with `~/.cargo/bin/cargo build --release -p soundcraft-bridge`.

```js
const bridge = spawn(binPath, ["--device", "MacBook Pro Speakers", "--buffer", "256"], { stdio: ["pipe", "pipe", "pipe"] });
```

- **stdin** takes one JSON request per line; **stdout** gives one JSON reply per line, in request order; **stderr** is the log. Read stdout line by line and match replies to requests by `id`. Requests may be pipelined.
- **Closing stdin ends the bridge** (exit 0), also with `--control`: the spawning process owns its lifetime, and an orphaned bridge never holds the audio device. `bridge.quit` ends it too, after its reply.
- Startup to the first reply is about 15 ms with `--no-audio` and about 0.6 s when opening a Bluetooth output. The first run of a freshly built binary takes about 0.5 s longer while macOS scans it.
- Third-party plugins run in the bridge's process; one that prints to stdout would put a non-JSON line on the protocol channel. A reader of stdout therefore skips any line that does not parse as a JSON object with an `id`, and logs it.

## Flags

| Flag | Meaning |
|---|---|
| `--device NAME` | The output device, by its exact name as `devices` lists it. Default: the system's default output. An unknown name exits 3 with the list of outputs on stderr. |
| `--buffer FRAMES` | The device buffer in frames: 16 to 8192 and inside the device's range (`buffer_min` to `buffer_max` in `devices`). Default: the device's own. A buffer the device refuses exits 3. |
| `--no-audio` | Open no device. The transport and the mix run in real time on upstream's silent clock. Takes no `--device` or `--buffer`. |
| `--control PORT` | Also serve the protocol on `127.0.0.1:PORT`; `0` picks a free port, announced on stderr as `SOUNDCRAFT_CONTROL_PORT=<port>`. |
| `--help`, `--version` | Print and exit 0. |

`SOUNDCRAFT_BRIDGE_LOG` sets the log level: `error`, `warn`, `info` (default), `debug`, `trace` or `off`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | stdin closed, or `bridge.quit` |
| 1 | stdin or stdout failed (the reason is on stderr) |
| 2 | bad arguments |
| 3 | the audio output was refused: unknown device, buffer out of range or refused by the device, or no output could be opened at all (the bridge never falls back to silence on its own) |
| 4 | the control port could not be bound |

## Wire format

```json
{"id": 1, "method": "engine.execute", "params": {"command": "track.new", "params": {"count": 2, "format": "stereo"}}}
{"id": 1, "ok": true, "result": {"tracks": [2, 3]}}
{"id": 2, "ok": false, "error": "bad parameters for `mix.volume`: no track named `Nope`"}
```

`id` is any JSON value and comes back unchanged. `params` may be left out. A line that is not JSON gets `{"id": null, "ok": false, "error": "bad JSON: ..."}`. A request line is at most about 86 MiB (room for the largest plugin state).

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` (alias `command`) | `{command, params?}` | The command's result. Every engine command (about 470: `session.new`, `file.import_audio`, `mix.volume`, `file.bounce_mix`, ...); `engine.commands` lists them with their params. Transport commands reach the player before the reply. |
| `engine.commands` | `{filter?}` | Every command: id, label, menu path, shortcut, params doc, enabled. |
| `engine.parity` | `{}` | Upstream's parity report. |
| `session.inspect` (alias `document.inspect`) | `{detail?: "summary" \| "full"}` | The document: tracks, clips, inserts, sends, routing, automation, markers, sources, selection, transport. |
| `transport.status` | `{}` | See below. |
| `transport.play` | `{from?: position}` | `{}`. Plays from `from`, else from the edit selection: a selection range plays to its end (with pre and post roll when on) or loops with loop playback; an insertion point plays on until `transport.stop`. |
| `transport.stop` | `{}` | `{}`. The position stays where playback stopped. |
| `transport.locate` | `{at: position}` | `{"at": samples}`. Moves the playhead, while playing too. A selection that contains `at` is kept; otherwise the edit insertion point moves to `at`. |
| `transport.toggle`, `transport.pause`, `transport.play_selection`, `transport.half_speed`, `transport.rtz`, `transport.go_to_end`, `transport.rewind {seconds?}`, `transport.fast_forward {seconds?}` | as the engine commands | as the engine commands |
| `transport.record` | | Refused: the bridge does not record yet. |
| `devices` (alias `audio.devices`) | `{}` | `{system, outputs: [{name, is_default, max_channels, sample_rates, buffer_min, buffer_max, buffer_sizes}]}` for the default audio system (CoreAudio on macOS). |
| `bridge.quit` (alias `app.quit`) | `{}` | `{}`, then the bridge exits 0. |
| `ui.*` | | Refused: the bridge has no window. |

Any `transport.*` method other than `transport.status` is the engine command of that id with the request's params, so `{"method": "transport.play", "params": {"from": 0}}` and `{"method": "engine.execute", "params": {"command": "transport.play", "params": {"from": 0}}}` are the same request.

Positions in params are samples (`96000`), seconds (`{"seconds": 2.5}`) or a time string (`"bars_beats:5|1|000"`, `"min_secs:0:12.500"`, `"timecode:00:00:10:00"`, `"samples:48000"`).

### `transport.status`

```json
{
  "playing": true,
  "recording": false,
  "position": 72704,
  "seconds": 1.5147,
  "sample_rate": 48000,
  "speed": 1.0,
  "device": {"name": "MacBook Pro Speakers", "sample_rate": 44100, "buffer": 256, "channels": 2, "silent": false},
  "trouble": null
}
```

- `position` is in **session samples** (at `sample_rate`, the session's rate), read from the player. While playing it advances in steps of the mix block, 512 session samples (about 10.7 ms at 48 kHz); poll it at the lab's frame rate and interpolate between steps by wall time if smoother motion is needed. Upstream's `engine.execute transport.status` reports the same `playing` and `position`.
- `device.sample_rate` is the rate the device runs at; the mix resamples when it differs from the session's. `device.buffer` is the buffer in frames when one was asked for and accepted, `null` when the device runs at its own default (cpal does not report that size). `device.silent` is true with `--no-audio`.
- `trouble` is `null`, or why the output stopped (the device was disconnected, stopped asking for audio, or the engine failed); the transport is stopped when it is set.
