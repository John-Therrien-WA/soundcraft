//! The output device and buffer the bridge plays through. Upstream's player falls back to the
//! default for a device or buffer it cannot honour; the bridge checks the choice against the
//! device list first and refuses what does not exist, naming what does.

use serde_json::{Value, json};
use soundcraft_playback::devices::{self, DeviceInfo};
use soundcraft_playback::{AudioConfig, Player};

/// Buffer sizes worth asking any device for (cpal's sane range, as upstream bounds it).
pub const BUFFER_RANGE: std::ops::RangeInclusive<u32> = 16..=8192;

/// The default audio system's name and output devices.
pub fn outputs() -> (String, Vec<DeviceInfo>) {
    let systems = devices::audio_systems();
    match systems.iter().position(|s| s.is_default).or(if systems.is_empty() { None } else { Some(0) }) {
        Some(i) => systems.into_iter().nth(i).map(|s| (s.name, s.outputs)).unwrap_or_default(),
        None => (String::new(), Vec::new()),
    }
}

fn device_json(d: &DeviceInfo) -> Value {
    json!({
        "name": d.name,
        "is_default": d.is_default,
        "max_channels": d.max_channels,
        "sample_rates": d.sample_rates,
        "buffer_min": d.buffer_min,
        "buffer_max": d.buffer_max,
        "buffer_sizes": d.buffer_sizes(),
    })
}

/// The `devices` method: the default audio system and its output devices.
pub fn devices_json() -> Value {
    let (system, outs) = outputs();
    json!({"system": system, "outputs": outs.iter().map(device_json).collect::<Vec<_>>()})
}

fn names(outs: &[DeviceInfo]) -> String {
    if outs.is_empty() { "there are none".into() } else { outs.iter().map(|d| format!("`{}`", d.name)).collect::<Vec<_>>().join(", ") }
}

/// Check `--device` and `--buffer` against `outs` before anything is opened.
pub fn choose(device: Option<&str>, buffer: Option<u32>, outs: &[DeviceInfo]) -> Result<AudioConfig, String> {
    let chosen = match device {
        Some(want) => {
            Some(outs.iter().find(|d| d.name == want).ok_or_else(|| format!("unknown output device `{want}`; the outputs are: {}", names(outs)))?)
        }
        None => outs.iter().find(|d| d.is_default),
    };
    if let Some(n) = buffer {
        if !BUFFER_RANGE.contains(&n) {
            return Err(format!("buffer {n} is outside {} to {} frames", BUFFER_RANGE.start(), BUFFER_RANGE.end()));
        }
        if let Some(d) = chosen
            && (d.buffer_min.is_some_and(|m| n < m) || d.buffer_max.is_some_and(|m| n > m))
        {
            let (lo, hi) = (d.buffer_min.map_or("?".into(), |v| v.to_string()), d.buffer_max.map_or("?".into(), |v| v.to_string()));
            return Err(format!("`{}` takes buffers of {lo} to {hi} frames, not {n}", d.name));
        }
    }
    Ok(AudioConfig { output: device.map(str::to_string), buffer_size: buffer, ..AudioConfig::default() })
}

/// Check what the player opened against what was asked: no silent fallback.
pub fn check_opened(player: &Player, cfg: &AudioConfig) -> Result<(), String> {
    if player.silent {
        return Err("no audio output could be opened (pass --no-audio for the silent clock)".into());
    }
    if let Some(e) = &player.start_error {
        return Err(format!("`{}` was opened but would not start: {e}", player.device_name));
    }
    if let Some(want) = &cfg.output
        && &player.device_name != want
    {
        return Err(format!("asked for `{want}`, opened `{}`", player.device_name));
    }
    if let Some(n) = cfg.buffer_size
        && player.device_buffer != Some(n)
    {
        return Err(format!("`{}` refused a buffer of {n} frames", player.device_name));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dev(name: &str, is_default: bool, range: Option<(u32, u32)>) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            is_default,
            max_channels: 2,
            sample_rates: vec![48_000],
            buffer_min: range.map(|r| r.0),
            buffer_max: range.map(|r| r.1),
        }
    }

    #[test]
    fn devices_are_chosen_by_name_and_refused_with_the_list() {
        let outs = [dev("Speakers", true, Some((32, 4096))), dev("Interface", false, Some((64, 1024)))];
        assert_eq!(choose(None, None, &outs).unwrap(), AudioConfig::default());
        let c = choose(Some("Interface"), Some(128), &outs).unwrap();
        assert_eq!((c.output.as_deref(), c.buffer_size), (Some("Interface"), Some(128)));
        let e = choose(Some("Nope"), None, &outs).unwrap_err();
        assert!(e.contains("`Nope`") && e.contains("`Speakers`, `Interface`"), "{e}");
        assert!(choose(Some("Nope"), None, &[]).unwrap_err().contains("there are none"));
    }

    #[test]
    fn buffers_are_checked_against_the_device_range() {
        let outs = [dev("Speakers", true, Some((32, 4096))), dev("Interface", false, Some((64, 1024))), dev("Quiet", false, None)];
        assert!(choose(None, Some(32), &outs).is_ok(), "the default device's range applies");
        assert!(choose(Some("Interface"), Some(32), &outs).unwrap_err().contains("64 to 1024"));
        assert!(choose(Some("Interface"), Some(2048), &outs).is_err());
        assert!(choose(Some("Quiet"), Some(256), &outs).is_ok(), "an unknown range is asked anyway");
        assert!(choose(None, Some(8), &outs).is_err());
        assert!(choose(None, Some(100_000), &outs).is_err());
    }

    #[test]
    fn a_silent_player_is_not_an_opened_device() {
        let s = std::sync::Arc::new(soundcraft_engine::model::Session::new("x", soundcraft_time::SampleRate::new(48_000).unwrap()));
        let e = check_opened(&Player::silent(s), &AudioConfig::default()).unwrap_err();
        assert!(e.contains("--no-audio"), "{e}");
    }

    #[test]
    fn the_device_list_has_names() {
        let v = devices_json();
        assert!(v["outputs"].as_array().unwrap().iter().all(|d| d["name"].as_str().is_some_and(|n| !n.is_empty())), "{v}");
    }
}
