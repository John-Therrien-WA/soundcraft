//! The session's tempo map reaches a plugin of ours end to end: a `TempoAware` plugin, registered
//! like the catalogue and inserted on a track by id, receives the map through
//! `forward_tempo_map` when `MixEngine` renders it, and again when the session's map changes.
//! A test binary of its own, because the registry is process-wide.

use std::any::Any;
use std::sync::{Arc, Mutex};

use soundcraft_dsp::{Category, Plugin, PluginInfo};
use soundcraft_mix::MixEngine;
use soundcraft_model::{ChannelFormat, Insert, Session, TrackKind};
use soundcraft_time::TempoMap;
use wild_animal_plugins::{TempoAware, forward_tempo_map};

static INFO: PluginInfo = PluginInfo {
    id: "test_wa_tempo",
    name: "Tempo Follower",
    short_name: "Tempo",
    category: Category::Other,
    params: &[],
    is_instrument: false,
    audiosuite: false,
};

/// Every map any instance received, in order.
static RECEIVED: Mutex<Vec<TempoMap>> = Mutex::new(Vec::new());

struct Follower;

impl TempoAware for Follower {
    fn set_tempo_map(&mut self, map: Arc<TempoMap>) {
        RECEIVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((*map).clone());
    }
}

impl Plugin for Follower {
    fn info(&self) -> &'static PluginInfo {
        &INFO
    }
    fn prepare(&mut self, _sample_rate: f32, _max_block: usize, _channels: usize) {}
    fn reset(&mut self) {}
    fn set_param(&mut self, _id: &str, _value: f32) -> bool {
        false
    }
    fn param(&self, _id: &str) -> Option<f32> {
        None
    }
    fn process(&mut self, _io: &mut [Vec<f32>], _frames: usize) {}
    fn set_tempo_map(&mut self, map: &Arc<dyn Any + Send + Sync>) {
        forward_tempo_map(self, map);
    }
}

fn make() -> Box<dyn Plugin> {
    Box::new(Follower)
}

fn received() -> Vec<TempoMap> {
    RECEIVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

#[test]
fn the_session_tempo_map_reaches_a_tempo_aware_plugin_through_the_mixer() {
    soundcraft_dsp::register_external(&INFO, make).unwrap();
    let mut s = Session::default();
    s.tempo.set_tempo(0, 93.0).unwrap();
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("test_wa_tempo"));

    let mut eng = MixEngine::new(48_000.0, 512);
    let mut out = vec![vec![0.0f32; 512]; 2];
    eng.render(&s, 0, 512, &mut out);
    eng.render(&s, 512, 512, &mut out);
    let got = received();
    assert_eq!(got, vec![s.tempo.clone()], "the map arrives once when the insert enters the session");
    assert_eq!(got[0].tempo_at_tick(0), 93.0);

    s.tempo.set_tempo(3840, 140.0).unwrap();
    eng.render(&s, 1024, 512, &mut out);
    let got = received();
    assert_eq!(got.len(), 2, "once more when the map changes");
    assert_eq!(got[1], s.tempo);
    assert_eq!(got[1].tempo_at_tick(3840), 140.0);
}
