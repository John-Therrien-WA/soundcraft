//! A plugin implemented outside the crate joins the registry: listed after the built-ins,
//! found by id, created and prepared by `create`, and refused on a duplicate id. A factory
//! may read the registry itself without deadlocking the registration.

use soundcraft_dsp::{Category, ParamInfo, Plugin, PluginInfo, RegisterError, Taper, Unit};

static PARAMS: [ParamInfo; 1] =
    [ParamInfo { id: "gain", name: "Gain", min: 0.0, max: 2.0, default: 1.0, unit: Unit::None, taper: Taper::Linear, choices: &[] }];

const BASE: PluginInfo = PluginInfo {
    id: "test_external_gain",
    name: "External Gain",
    short_name: "ExtGain",
    category: Category::Other,
    params: &PARAMS,
    is_instrument: false,
    audiosuite: false,
};
static INFO: PluginInfo = BASE;
static DUPLICATE: PluginInfo = PluginInfo { id: "eq_1band", ..BASE };
static OTHER: PluginInfo = PluginInfo { id: "test_external_other", ..BASE };
static READER: PluginInfo = PluginInfo { id: "test_external_reader", ..BASE };

struct ExternalGain {
    info: &'static PluginInfo,
    gain: f32,
    prepared: usize,
}

impl Plugin for ExternalGain {
    fn info(&self) -> &'static PluginInfo {
        self.info
    }
    fn prepare(&mut self, _sample_rate: f32, _max_block: usize, channels: usize) {
        self.prepared = channels;
    }
    fn reset(&mut self) {}
    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if id == "gain" {
            self.gain = PARAMS[0].clamp(value);
            true
        } else {
            false
        }
    }
    fn param(&self, id: &str) -> Option<f32> {
        (id == "gain").then_some(self.gain)
    }
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        for ch in io.iter_mut().take(self.prepared) {
            ch.iter_mut().take(frames).for_each(|x| *x *= self.gain);
        }
    }
}

fn make() -> Box<dyn Plugin> {
    Box::new(ExternalGain { info: &INFO, gain: 1.0, prepared: 0 })
}

/// A factory that reads the registry while it builds its plugin.
fn make_reading() -> Box<dyn Plugin> {
    assert!(soundcraft_dsp::plugin_info("gain").is_some(), "the factory sees the built-ins");
    Box::new(ExternalGain { info: &READER, gain: 1.0, prepared: 0 })
}

fn make_wrong() -> Box<dyn Plugin> {
    soundcraft_dsp::create("gain").unwrap_or_else(make)
}

// One test: the registry is process-wide, so the steps run in order in one thread.
#[test]
fn an_external_plugin_is_listed_found_and_created() {
    let builtins = soundcraft_dsp::plugins().len();
    assert!(soundcraft_dsp::plugin_info("test_external_gain").is_none());

    soundcraft_dsp::register_external(&INFO, make).unwrap();
    let list = soundcraft_dsp::plugins();
    assert_eq!(list.len(), builtins + 1);
    assert_eq!(list.last().map(|i| i.id), Some("test_external_gain"));
    assert_eq!(list.first().map(|i| i.id), Some("eq_1band"), "built-ins keep their order in front");
    assert!(std::ptr::eq(soundcraft_dsp::plugin_info("test_external_gain").unwrap(), &INFO));

    let mut p = soundcraft_dsp::create("test_external_gain").unwrap();
    assert!(p.set_param("gain", 0.5));
    let mut io = vec![vec![1.0f32; 4]; 2];
    p.process(&mut io, 4);
    assert_eq!(io, vec![vec![0.5f32; 4]; 2], "create prepares the instance (stereo)");
    assert!(soundcraft_dsp::create("compressor").is_some(), "built-ins still instantiate");

    assert_eq!(soundcraft_dsp::register_external(&INFO, make), Err(RegisterError::DuplicateId("test_external_gain")));
    assert_eq!(soundcraft_dsp::register_external(&DUPLICATE, make), Err(RegisterError::DuplicateId("eq_1band")));
    assert_eq!(soundcraft_dsp::register_external(&OTHER, make_wrong), Err(RegisterError::InfoMismatch("test_external_other")));
    assert_eq!(soundcraft_dsp::plugins().len(), builtins + 1, "refused registrations leave the list alone");

    // A factory that reads the registry: on its own thread, so a deadlock fails the test
    // instead of hanging it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(soundcraft_dsp::register_external(&READER, make_reading));
    });
    let result = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("register_external with a registry-reading factory completes");
    assert_eq!(result, Ok(()));
    assert_eq!(soundcraft_dsp::plugins().last().map(|i| i.id), Some("test_external_reader"));
}
