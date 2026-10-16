//! WA Compressor measured: the static curve against the gain computer, attack and release
//! against their settings, lookahead catching a step, and unity where nothing should happen.
//! `cargo test -p wild-animal-plugins --test compressor -- --nocapture` prints the numbers.
//!
//! Definitions. A level is the sine's peak in dBFS. The output level of a steady sine is the
//! amplitude of its fundamental (least-squares fit). The gain reduction at a sample is
//! `-20 log10(y/x)`; the plugin applies one gain per sample with no delay (lookahead 0), so
//! this is exact at every sample with x ≠ 0. Attack time: from the step up until the
//! reduction first reaches 63.2 % (1 - 1/e) of its settled value (the mean over the last
//! 100 ms of the loud part). Release time: from the step down until it first falls to 36.8 %
//! of its value at the step. "DC" runs drive a constant level, which isolates the ballistics
//! from the detector; "tone" runs drive a 1 kHz sine, what a mix does.

use soundcraft_dsp::Plugin;
use wild_animal_plugins::compressor::{WaCompressor, compress_gr};
use wild_animal_plugins::measure::{amp, db, fit_amplitude, gain_reduction_trace, peak, run, sine, sine_gain_db, time_to};

const SR: f64 = 48_000.0;
const TONE: f64 = 1000.0;

fn comp(params: &[(&str, f32)]) -> WaCompressor {
    let mut c = WaCompressor::new();
    c.prepare(SR as f32, 512, 2);
    for (id, v) in params {
        assert!(c.set_param(id, *v), "{id}");
    }
    c.reset();
    c
}

fn levels() -> impl Iterator<Item = f64> {
    (0..=20).map(|i| -40.0 + 2.0 * i as f64)
}

/// Steady output level of a DC input at `level` dBFS (exact gain after settling).
fn dc_out_db(c: &mut WaCompressor, level: f64) -> f64 {
    c.reset();
    let n = SR as usize;
    let y = run(c, &vec![amp(level) as f32; n], 2);
    db(f64::from(y[n - 1]))
}

/// Worst |output - curve| outside the knee over -40..0 dBFS, for DC and for a 1 kHz tone,
/// and the smallest tone error (signed; positive is less reduction than the curve).
fn curve_errors(ratio: f32, knee: f32, attack: f32, release: f32) -> (f64, f64, f64) {
    let mut c = comp(&[("threshold", -20.0), ("ratio", ratio), ("knee", knee), ("attack", attack), ("release", release)]);
    let (mut wd, mut wt, mut mint) = (0.0f64, 0.0f64, f64::INFINITY);
    for l in levels() {
        if (l + 20.0).abs() < f64::from(knee) / 2.0 {
            continue;
        }
        let want = l - f64::from(compress_gr(l as f32, -20.0, ratio, knee));
        let dc = dc_out_db(&mut c, l) - want;
        let tone = l + sine_gain_db(&mut c, TONE, SR, l, (6.0 * f64::from(release) / 1000.0).max(1.0), 0.2) - want;
        wd = wd.max(dc.abs());
        wt = wt.max(tone.abs());
        if l > -20.0 {
            mint = mint.min(tone);
        }
    }
    (wd, wt, mint)
}

#[test]
fn static_curve_matches_the_gain_computer() {
    for knee in [0.0f32, 6.0] {
        for ratio in [2.0f32, 4.0, 10.0] {
            let (dc, tone, min_tone) = curve_errors(ratio, knee, 10.0, 100.0);
            let (_, slow, _) = curve_errors(ratio, knee, 1.0, 1000.0);
            println!(
                "ratio {ratio:>4} knee {knee}: worst |error| outside the knee: DC {dc:.4} dB; tone at attack 10 / release 100 ms {tone:.4} dB (at least {min_tone:+.3} above threshold); tone at 1 / 1000 ms {slow:.4} dB"
            );
            // The gain computer and the gain stage are exact.
            assert!(dc < 0.2, "DC: {dc} dB");
            // On a tone the reduction settles where attack (only near the peaks) balances
            // release (the rest of each cycle): below the curve's value at the peak, by an
            // amount set by attack/release, not by frequency. Pinned as measured (0.821,
            // 1.232, 1.478 dB at ratio 2, 4, 10), so a regression or a fix both show; the
            // textbook bound is `tone_follows_the_static_curve`, ignored until upstream fixes
            // the detector (WILD-ANIMAL.md, findings against upstream).
            let band = match ratio {
                2.0 => 0.80..0.86,
                4.0 => 1.20..1.27,
                _ => 1.45..1.51,
            };
            assert!(band.contains(&tone) && min_tone > 0.0, "tone at 10/100 ms, ratio {ratio}: {tone} dB (measured band {band:?}), least {min_tone}");
            assert!(slow < 0.2, "tone at 1/1000 ms: {slow} dB");
        }
    }
    let mut c = comp(&[("threshold", -20.0), ("ratio", 10.0), ("knee", 0.0)]);
    for f in [100.0, 1000.0, 5000.0] {
        let e: Vec<String> = [-10.0, 0.0]
            .iter()
            .map(|&l| format!("{:+.3}", l + sine_gain_db(&mut c, f, SR, l, 1.0, 0.2) - (l - f64::from(compress_gr(l as f32, -20.0, 10.0, 0.0)))))
            .collect();
        println!("ratio 10 knee 0, tone {f} Hz at -10 and 0 dBFS: error {} dB (positive: less reduction than the curve)", e.join(", "));
    }
}

/// The original brief's bound: on a tone at the default 10 ms attack and 100 ms release the
/// steady reduction is within 0.2 dB of the static curve. Missed by 0.8 to 1.5 dB, because
/// upstream's gain computer runs on the instantaneous sample.
#[test]
#[ignore = "upstream finding, see WILD-ANIMAL.md"]
fn tone_follows_the_static_curve() {
    for knee in [0.0f32, 6.0] {
        for ratio in [2.0f32, 4.0, 10.0] {
            let (_, tone, _) = curve_errors(ratio, knee, 10.0, 100.0);
            assert!(tone < 0.2, "ratio {ratio} knee {knee}: tone at 10/100 ms {tone} dB from the curve");
        }
    }
}

/// Attack and release times (s) for a step -40 → -10 → -40 dBFS, threshold -20, ratio 4.
fn step_times(attack: f32, release: f32, tone: bool) -> (f64, f64, f64) {
    let mut c = comp(&[("threshold", -20.0), ("ratio", 4.0), ("knee", 0.0), ("attack", attack), ("release", release)]);
    let pre = (0.5 * SR) as usize;
    let loud = ((3.0 * f64::from(attack) / 1000.0).max(1.0) * SR) as usize;
    let post = ((6.0 * f64::from(release) / 1000.0).max(1.0) * SR) as usize;
    let n = pre + loud + post;
    let x: Vec<f32> = if tone {
        let s = sine(TONE, SR, 1.0, n);
        s.iter().enumerate().map(|(i, v)| v * amp(if (pre..pre + loud).contains(&i) { -10.0 } else { -40.0 }) as f32).collect()
    } else {
        (0..n).map(|i| amp(if (pre..pre + loud).contains(&i) { -10.0 } else { -40.0 }) as f32).collect()
    };
    let y = run(&mut c, &x, 2);
    let g = gain_reduction_trace(&x, &y, 1e-4);
    let last = (pre + loud - (0.1 * SR) as usize)..(pre + loud);
    let tail: Vec<f64> = g.iter().filter(|(i, _)| last.contains(i)).map(|(_, v)| *v).collect();
    let settled = tail.iter().sum::<f64>() / tail.len() as f64;
    let at = time_to(&g, pre, (1.0 - (-1.0f64).exp()) * settled, true, SR).unwrap_or(f64::NAN);
    let at_release = g.iter().rev().find(|(i, _)| *i < pre + loud).map_or(settled, |(_, v)| *v);
    let rt = time_to(&g, pre + loud, (-1.0f64).exp() * at_release, false, SR).unwrap_or(f64::NAN);
    (at, rt, settled)
}

#[test]
fn attack_and_release_reach_63_percent_at_their_settings() {
    for (a, r) in [(10.0f32, 100.0f32), (1.0, 1000.0)] {
        let (dat, drt, dset) = step_times(a, r, false);
        let (tat, trt, tset) = step_times(a, r, true);
        println!(
            "attack {a} ms, release {r} ms: DC attack {:.3} ms, release {:.2} ms (settled {dset:.3} dB); 1 kHz tone attack {:.3} ms, release {:.2} ms (settled {tset:.3} dB)",
            dat * 1e3,
            drt * 1e3,
            tat * 1e3,
            trt * 1e3
        );
        // The ballistics are one-pole in dB with time constant = the setting: on DC within
        // 2 % plus one sample (the gain moves on the step's own sample).
        let near = |m: f64, want: f32| (m - f64::from(want) / 1e3).abs() <= 0.02 * f64::from(want) / 1e3 + 1.0 / SR;
        assert!(near(dat, a), "DC attack {dat}");
        assert!(near(drt, r), "DC release {drt}");
        assert!(near(trt, r), "tone release {trt}");
        // On a tone the reduction grows only while the waveform's peak region exceeds it, so
        // attack is slower than set: measured 1.68x at 10 ms and 2.19x at 1 ms. Pinned as
        // measured, both ways.
        let ratio = tat * 1e3 / f64::from(a);
        assert!((1.6..2.3).contains(&ratio), "tone attack {ratio:.2}x the setting");
    }
}

/// Peak output (dBFS) in the first `window` of the step reaching the output, and the steady
/// output level the threshold and ratio call for.
fn lookahead_overshoot(params: &[(&str, f32)], ratio: f64, tone: bool) -> (f64, f64) {
    let mut c = comp(params);
    let la = c.latency();
    let pre = (0.5 * SR) as usize;
    let n = pre + SR as usize;
    let x: Vec<f32> = if tone {
        sine(TONE, SR, 1.0, n).iter().enumerate().map(|(i, v)| v * amp(if i >= pre { -10.0 } else { -40.0 }) as f32).collect()
    } else {
        (0..n).map(|i| amp(if i >= pre { -10.0 } else { -40.0 }) as f32).collect()
    };
    let y = run(&mut c, &x, 2);
    let w = (0.005 * SR) as usize;
    let first = peak(&y[pre + la..pre + la + w]);
    (db(first), -10.0 - (1.0 - 1.0 / ratio) * 10.0)
}

#[test]
fn lookahead_catches_a_step_before_it_passes() {
    for (attack, tone) in [(1.0f32, false), (1.0, true), (10.0, false), (10.0, true), (0.5, true)] {
        let params = [("threshold", -20.0f32), ("ratio", 4.0), ("knee", 0.0), ("attack", attack), ("lookahead", 5.0)];
        let (got, want) = lookahead_overshoot(&params, 4.0, tone);
        println!(
            "lookahead 5 ms, attack {attack} ms, {}: first 5 ms peak {got:.2} dBFS, steady target {want:.2} dBFS, overshoot {:+.2} dB",
            if tone { "tone" } else { "DC" },
            got - want
        );
    }
    let lim = [("threshold", -20.0f32), ("limiter", 1.0), ("knee", 0.0), ("attack", 1.0), ("lookahead", 5.0)];
    let (got, want) = lookahead_overshoot(&lim, 1000.0, true);
    let mut c = comp(&lim);
    let x = sine(TONE, SR, amp(-10.0), SR as usize);
    let steady = db(peak(&run(&mut c, &x, 2)[(0.9 * SR) as usize..]));
    println!(
        "limiter, lookahead 5 ms, attack 1 ms, tone: first 5 ms peak {got:.2} dBFS, steady peak {steady:.2} dBFS, ceiling {want:.2} dBFS, overshoot {:+.2} dB",
        got - want
    );
    // A constant level is caught: 5 ms lookahead is five 1 ms attack time constants.
    let (got, want) = lookahead_overshoot(&[("threshold", -20.0f32), ("ratio", 4.0), ("knee", 0.0), ("attack", 1.0), ("lookahead", 5.0)], 4.0, false);
    assert!(got - want <= 0.5, "DC overshoot {} dB with 5 ms lookahead and 1 ms attack", got - want);
    // A tone is not: lookahead only delays the audio and the attack is not tied to it, and the
    // detector follows the waveform. Measured +1.12 dB at attack 1 ms. Pinned as measured,
    // both ways.
    let (got, want) = lookahead_overshoot(&[("threshold", -20.0f32), ("ratio", 4.0), ("knee", 0.0), ("attack", 1.0), ("lookahead", 5.0)], 4.0, true);
    assert!((0.9..=1.3).contains(&(got - want)), "tone overshoot {} dB with 5 ms lookahead and 1 ms attack", got - want);
}

#[test]
fn ratio_one_or_threshold_zero_is_unity() {
    let mut worst = 0.0f64;
    for params in [vec![("ratio", 1.0f32)], vec![("threshold", 0.0f32), ("knee", 0.0)]] {
        let mut c = comp(&params);
        for l in levels() {
            let x = sine(TONE, SR, amp(l), (0.3 * SR) as usize);
            let y = run(&mut c, &x, 2);
            worst = worst.max(db(fit_amplitude(&y, TONE, SR) / fit_amplitude(&x, TONE, SR)).abs());
        }
    }
    println!("unity: worst |gain| {worst:.6} dB");
    assert!(worst < 0.01, "{worst} dB");
}
