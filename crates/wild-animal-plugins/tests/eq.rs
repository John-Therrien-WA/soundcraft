//! WA EQ 7 measured: the running filter against its own analytic response, the analytic
//! response against the textbook landmarks (RBJ cookbook, Butterworth), unity when flat, and
//! silence after signal. `cargo test -p wild-animal-plugins --test eq -- --nocapture` prints
//! the tables.

use soundcraft_dsp::Plugin;
use wild_animal_plugins::eq::{WaEq7, wa_eq7_response};
use wild_animal_plugins::measure::{amp, db, peak, run, sine, sine_gain_db};

const SR: f64 = 48_000.0;
/// Agreement of the running filter with the analytic response, everywhere measured
/// (20 Hz to 20 kHz, that is up to 0.42 of the sample rate).
const TOL_DB: f64 = 0.1;

/// 24 frequencies, log-spaced from 20 Hz to 20 kHz.
fn freqs() -> Vec<f64> {
    (0..24).map(|i| 20.0 * 1000f64.powf(i as f64 / 23.0)).collect()
}

fn eq(params: &[(&str, f32)]) -> WaEq7 {
    let mut e = WaEq7::new();
    e.prepare(SR as f32, 512, 2);
    for (id, v) in params {
        assert!(e.set_param(id, *v), "{id}");
    }
    e.reset();
    e
}

fn analytic(params: &[(&str, f32)], f: f64) -> f64 {
    f64::from(wa_eq7_response(params, &[f as f32], SR as f32).unwrap()[0])
}

/// Measures every frequency and returns the worst |measured - analytic|, printing the table.
fn check_against_analytic(name: &str, params: &[(&str, f32)]) -> f64 {
    let mut e = eq(params);
    let mut worst = 0.0f64;
    println!("{name}: f Hz, measured dB, analytic dB, difference");
    for f in freqs() {
        let m = sine_gain_db(&mut e, f, SR, -12.0, 0.5, 0.25);
        let a = analytic(params, f);
        println!("  {f:8.1} {m:9.4} {a:9.4} {:+.4}", m - a);
        worst = worst.max((m - a).abs());
    }
    println!("  worst {worst:.4} dB");
    worst
}

const PEAK: &[(&str, f32)] = &[("mid_freq", 1000.0), ("mid_gain", 6.0), ("mid_q", 1.0)];
const LOW_SHELF: &[(&str, f32)] = &[("low_shelf_freq", 120.0), ("low_shelf_gain", -4.0)];
const HPF_SHELF: &[(&str, f32)] = &[("hpf_on", 1.0), ("hpf_freq", 80.0), ("hpf_slope", 3.0), ("high_shelf_freq", 8000.0), ("high_shelf_gain", 3.0)];

#[test]
fn flat_passes_unity() {
    let mut e = eq(&[]);
    let worst = freqs().into_iter().map(|f| sine_gain_db(&mut e, f, SR, -12.0, 0.05, 0.1).abs()).fold(0.0, f64::max);
    println!("flat: worst |gain| {worst:.6} dB");
    assert!(worst < 0.01, "flat EQ off unity by {worst} dB");
    // Flat is bit-transparent: every section is the identity and the gain is exactly 1.
    let x = sine(997.0, SR, 0.5, 4096);
    assert_eq!(run(&mut e, &x, 2), x);
}

#[test]
fn peak_6db_at_1k_q1_matches_analytic_and_cookbook() {
    assert!(check_against_analytic("peak +6 dB 1 kHz Q 1", PEAK) < TOL_DB);
    // Cookbook peak: the full gain at the centre, close to none far from it.
    assert!((analytic(PEAK, 1000.0) - 6.0).abs() < 0.001);
    assert!(analytic(PEAK, 20.0).abs() < 0.02 && analytic(PEAK, 20_000.0).abs() < 0.2);
}

#[test]
fn low_shelf_minus4_at_120_matches_analytic_and_cookbook() {
    assert!(check_against_analytic("low shelf -4 dB 120 Hz", LOW_SHELF) < TOL_DB);
    // Cookbook shelf: half the dB gain at f0, the full gain far below it, none far above.
    assert!((analytic(LOW_SHELF, 120.0) + 2.0).abs() < 0.001);
    assert!((analytic(LOW_SHELF, 20.0) + 4.0).abs() < 0.15);
    assert!(analytic(LOW_SHELF, 5000.0).abs() < 0.01);
}

#[test]
fn hpf_24_at_80_with_high_shelf_3_at_8k_matches_analytic_and_butterworth() {
    assert!(check_against_analytic("HPF 24 dB/oct 80 Hz + high shelf +3 dB 8 kHz", HPF_SHELF) < TOL_DB);
    // 4th-order Butterworth high-pass by the bilinear transform prewarped at fc:
    // |H| = 1 / sqrt(1 + (tan(π fc/fs) / tan(π f/fs))^8). The shelf adds ≈ 0 dB down here.
    let bw = |f: f64| -10.0 * (1.0 + ((std::f64::consts::PI * 80.0 / SR).tan() / (std::f64::consts::PI * f / SR).tan()).powi(8)).log10();
    for f in [20.0, 40.0, 80.0, 160.0] {
        let a = analytic(HPF_SHELF, f);
        println!("Butterworth at {f} Hz: analytic {a:.4} dB, textbook {:.4} dB, difference {:+.4}", bw(f), a - bw(f));
        assert!((a - bw(f)).abs() < 0.02, "{f} Hz: {a} vs {}", bw(f));
    }
    // Cookbook high shelf: half its dB gain at f0, the full gain at Nyquist.
    let shelf_only: &[(&str, f32)] = &[("high_shelf_freq", 8000.0), ("high_shelf_gain", 3.0)];
    assert!((analytic(shelf_only, 8000.0) - 1.5).abs() < 0.001);
    assert!((analytic(shelf_only, 23_999.0) - 3.0).abs() < 0.01);
}

#[test]
fn silence_after_signal_decays_below_minus_120_dbfs_without_denormals() {
    for (name, params) in [("peak", PEAK), ("low shelf", LOW_SHELF), ("hpf + shelf", HPF_SHELF)] {
        let mut e = eq(params);
        let n = SR as usize;
        let mut x: Vec<f32> =
            (0..n).map(|i| (0.9 * (std::f64::consts::TAU * (20.0 + 1000.0 * i as f64 / n as f64) * i as f64 / SR).sin()) as f32).collect();
        x.extend(std::iter::repeat_n(0.0f32, 10 * n));
        let y = run(&mut e, &x, 2);
        let tail = &y[n..];
        assert!(tail.iter().all(|v| v.is_finite()), "{name}: non-finite output");
        let subnormal = tail.iter().filter(|v| v.is_subnormal()).count();
        let below = tail.iter().rposition(|v| f64::from(v.abs()) >= amp(-120.0)).map_or(0, |i| i + 1);
        let last = peak(&tail[9 * n..]);
        println!(
            "{name}: below -120 dBFS after {:.1} ms of silence; last second peak {:.1} dBFS; {subnormal} subnormal samples",
            below as f64 / SR * 1000.0,
            db(last)
        );
        assert_eq!(subnormal, 0, "{name}");
        assert!(db(last) < -120.0, "{name}: {} dBFS", db(last));
    }
}
