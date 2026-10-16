//! The plugin building blocks are usable from outside the crate, so a plugin written elsewhere
//! is built from the same parameter tables, smoothers and delay lines as the built-ins.

use soundcraft_dsp::params::{Params, lin, log, toggle};
use soundcraft_dsp::util::{DelayLine, Smoother, time_coef};
use soundcraft_dsp::{ParamInfo, Unit};

static PARAMS: [ParamInfo; 3] =
    [lin("gain", "Gain", -24.0, 24.0, 0.0, Unit::Db), log("freq", "Freq", 20.0, 20_000.0, 1000.0, Unit::Hz), toggle("on", "On", true)];

#[test]
fn params_clamp_and_report_like_the_built_ins() {
    let mut p = Params::new(&PARAMS);
    assert_eq!(p.set("gain", 99.0), Some(0));
    assert_eq!(p.get("gain"), Some(24.0));
    assert_eq!(p.set("nope", 1.0), None);
    assert!(p.on(2));
    assert_eq!(p.v(1), 1000.0);
}

#[test]
fn smoother_reaches_one_minus_one_over_e_after_its_time_constant() {
    let sr = 48_000.0;
    let mut s = Smoother::new(0.0);
    s.set_time(10.0, sr);
    s.set(1.0);
    let v = s.advance(480);
    assert!((v - (1.0 - (-1.0f32).exp())).abs() < 1e-4, "{v}");
    assert!((time_coef(10.0, sr) - (-1.0f32 / 480.0).exp()).abs() < 1e-7);
}

#[test]
fn delay_line_taps_count_back_from_the_last_push() {
    let mut d = DelayLine::default();
    d.allocate(8);
    for x in 1..=5 {
        d.push(x as f32);
    }
    assert_eq!((d.tap(1), d.tap(3)), (5.0, 3.0));
}
