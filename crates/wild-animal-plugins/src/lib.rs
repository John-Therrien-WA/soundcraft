#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Wild Animal's own plugins, inside the DAW: each implements `soundcraft_dsp::Plugin`, so the
//! engine lists, creates, automates and renders it like a built-in, and each can be edited and
//! forked here. [`register`] adds them to the DSP registry; call it once at program start.
//!
//! The catalogue opens with forks of two upstream built-ins, measured against the textbook in
//! `tests/`: [`eq::WaEq7`] (`wa_eq7`) and [`compressor::WaCompressor`] (`wa_compressor`).
//! [`measure`] is the harness those tests use, for every later plugin too.
//!
//! Plugins that follow tempo read the session's tempo map as shared structure through
//! [`TempoAware`], never only the per-block `Transport.tempo`.

pub mod compressor;
pub mod eq;
pub mod measure;

use std::any::Any;
use std::sync::{Arc, Once};

use soundcraft_dsp::{Plugin, PluginFactory, PluginInfo};
use soundcraft_time::TempoMap;

/// Every plugin of this crate, in menu order.
pub const CATALOGUE: &[(&PluginInfo, PluginFactory)] = &[(&eq::WA_EQ7_INFO, new_eq7), (&compressor::WA_COMP_INFO, new_compressor)];

fn new_eq7() -> Box<dyn Plugin> {
    Box::new(eq::WaEq7::new())
}

fn new_compressor() -> Box<dyn Plugin> {
    Box::new(compressor::WaCompressor::new())
}

/// Adds [`CATALOGUE`] to the DSP registry, so `soundcraft_dsp::plugins()`, `plugin_info()` and
/// `create()` (and the engine's `engine.plugins`, insert creation, offline rendering) know them.
/// Idempotent; the first call registers. An id clash with another plugin is a build error of
/// ours, reported as `Err` with the id.
pub fn register() -> Result<(), String> {
    static ONCE: Once = Once::new();
    let mut result = Ok(());
    ONCE.call_once(|| {
        for (info, factory) in CATALOGUE {
            if let Err(e) = soundcraft_dsp::register_external(info, *factory) {
                result = Err(format!("wild-animal-plugins: {e}"));
                return;
            }
        }
    });
    result
}

/// A plugin that reads the session's tempo map directly. The engine hands the map over when
/// the instance enters the session and again whenever the map changes, always between blocks.
pub trait TempoAware {
    fn set_tempo_map(&mut self, map: Arc<TempoMap>);
}

/// Forwards the DSP trait's type-erased `Plugin::set_tempo_map` to [`TempoAware`]: the body of
/// that method for every tempo-aware plugin here. A host handing anything but a
/// `soundcraft_time::TempoMap` is a host bug; it is logged as an error and the plugin keeps
/// its previous map. Returns whether the map arrived.
pub fn forward_tempo_map<P: TempoAware + ?Sized>(plugin: &mut P, map: &Arc<dyn Any + Send + Sync>) -> bool {
    match Arc::clone(map).downcast::<TempoMap>() {
        Ok(m) => {
            plugin.set_tempo_map(m);
            true
        }
        Err(_) => {
            log::error!("wild-animal-plugins: the host handed a tempo map that is not a soundcraft_time::TempoMap");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Follower {
        map: Option<Arc<TempoMap>>,
    }

    impl TempoAware for Follower {
        fn set_tempo_map(&mut self, map: Arc<TempoMap>) {
            self.map = Some(map);
        }
    }

    #[test]
    fn the_erased_map_reaches_a_tempo_aware_plugin() {
        let mut tm = TempoMap::default();
        assert!(tm.set_tempo(0, 93.0).is_ok());
        let erased: Arc<dyn Any + Send + Sync> = Arc::new(tm.clone());
        let mut f = Follower::default();
        assert!(forward_tempo_map(&mut f, &erased));
        assert_eq!(f.map.as_deref(), Some(&tm));
        let wrong: Arc<dyn Any + Send + Sync> = Arc::new(120.0f64);
        assert!(!forward_tempo_map(&mut f, &wrong));
        assert_eq!(f.map.as_deref(), Some(&tm), "a wrong type leaves the previous map");
    }

    #[test]
    fn register_lists_and_creates_both_and_is_idempotent() {
        assert_eq!(register(), Ok(()));
        assert_eq!(register(), Ok(()));
        let ids: Vec<&str> = soundcraft_dsp::plugins().iter().map(|i| i.id).collect();
        for id in ["wa_eq7", "wa_compressor"] {
            assert_eq!(ids.iter().filter(|i| **i == id).count(), 1, "{id} listed once");
            let p = soundcraft_dsp::create(id);
            assert_eq!(p.map(|p| p.info().id), Some(id));
        }
        assert!(ids.contains(&"eq_7band") && ids.contains(&"compressor"), "upstream's stay beside ours");
    }
}
