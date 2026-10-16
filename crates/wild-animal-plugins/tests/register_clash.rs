//! A clash is reported on every call to `register`, not only the first: a plugin already holds
//! the id `wa_eq7` when the catalogue registers. A test binary of its own, because the registry
//! is process-wide.

use soundcraft_dsp::{Category, Plugin, PluginInfo};

static SQUATTER: PluginInfo = PluginInfo {
    id: "wa_eq7",
    name: "Squatter",
    short_name: "Squat",
    category: Category::Other,
    params: &[],
    is_instrument: false,
    audiosuite: false,
};

struct Squatter;

impl Plugin for Squatter {
    fn info(&self) -> &'static PluginInfo {
        &SQUATTER
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
}

fn make() -> Box<dyn Plugin> {
    Box::new(Squatter)
}

#[test]
fn a_clash_is_an_error_on_every_call() {
    assert_eq!(soundcraft_dsp::register_external(&SQUATTER, make), Ok(()));
    let first = wild_animal_plugins::register();
    let second = wild_animal_plugins::register();
    assert!(first.as_ref().is_err_and(|e| e.contains("wa_eq7")), "first call: {first:?}");
    assert_eq!(second, first, "the second call returns the stored error");
}
