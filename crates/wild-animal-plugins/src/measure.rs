//! The measurement harness for this crate's plugins: drive a plugin with test signals and read
//! steady-state gains, levels and gain trajectories back. Every function works on a
//! `Box<dyn Plugin>` (or `&mut dyn Plugin`), so a later plugin's tests use it unchanged.
//!
//! Levels are in dBFS of the sine's peak (a full-scale sine is 0 dBFS). Gains of a sine are
//! read by a least-squares fit of a sine and cosine at the known frequency over a window of
//! the settled output, which needs no whole number of periods and ignores harmonics and noise
//! orthogonal to the fundamental.

use soundcraft_dsp::Plugin;

/// Block size every helper processes in.
pub const BLOCK: usize = 512;

/// `n` samples of `amp · sin(2π f t)`.
pub fn sine(freq: f64, sr: f64, amp: f64, n: usize) -> Vec<f32> {
    (0..n).map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / sr).sin()) as f32).collect()
}

/// Linear amplitude of a level in dBFS.
pub fn amp(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// dB of a linear amplitude (floored at -400 dB).
pub fn db(a: f64) -> f64 {
    20.0 * a.abs().max(1e-20).log10()
}

/// Runs `input` through the plugin on `channels` identical channels in blocks of [`BLOCK`];
/// returns channel 0 of the output.
pub fn run(plugin: &mut dyn Plugin, input: &[f32], channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(input.len());
    let mut io = vec![vec![0.0f32; BLOCK]; channels];
    for chunk in input.chunks(BLOCK) {
        for ch in io.iter_mut() {
            ch[..chunk.len()].copy_from_slice(chunk);
        }
        plugin.process(&mut io, chunk.len());
        out.extend_from_slice(&io[0][..chunk.len()]);
    }
    out
}

/// Amplitude of the component at `freq` in `x` (least-squares fit of `a sin + b cos`).
pub fn fit_amplitude(x: &[f32], freq: f64, sr: f64) -> f64 {
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (std::f64::consts::TAU * freq * i as f64 / sr).sin_cos();
        let v = f64::from(v);
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v * s;
        xc += v * c;
    }
    let det = ss * cc - sc * sc;
    if det.abs() < 1e-12 {
        return 0.0;
    }
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    a.hypot(b)
}

/// Steady-state gain in dB of the plugin for a sine at `freq` and level `level_db`: the plugin
/// is reset, driven for `settle_s` seconds, and the gain read over the next `window_s`
/// seconds (the fit runs on the output and the input over the same samples).
pub fn sine_gain_db(plugin: &mut dyn Plugin, freq: f64, sr: f64, level_db: f64, settle_s: f64, window_s: f64) -> f64 {
    plugin.reset();
    let settle = (settle_s * sr) as usize;
    let window = ((window_s * sr) as usize).max(16);
    let x = sine(freq, sr, amp(level_db), settle + window);
    let y = run(plugin, &x, 2);
    let a_in = fit_amplitude(&x[settle..], freq, sr);
    let a_out = fit_amplitude(&y[settle..], freq, sr);
    db(a_out / a_in)
}

/// Largest absolute sample of `x`.
pub fn peak(x: &[f32]) -> f64 {
    x.iter().fold(0.0f64, |m, v| m.max(f64::from(v.abs())))
}

/// Time in seconds, from index `from`, until the trajectory `g` first reaches `target` going
/// in the direction of `rising`; `None` if it never does. Fractional by linear interpolation.
pub fn time_to(g: &[(usize, f64)], from: usize, target: f64, rising: bool, sr: f64) -> Option<f64> {
    let mut prev: Option<(usize, f64)> = None;
    for &(i, v) in g.iter().filter(|(i, _)| *i >= from) {
        let hit = if rising { v >= target } else { v <= target };
        if hit {
            let t = match prev {
                Some((pi, pv)) if (v - pv).abs() > 1e-12 => pi as f64 + (target - pv) / (v - pv) * (i - pi) as f64,
                _ => i as f64,
            };
            return Some((t - from as f64) / sr);
        }
        prev = Some((i, v));
    }
    None
}

/// The gain reduction in dB per sample, `-20 log10(y / x)`, at the samples where the input is
/// at least `min_abs` (a sine's zero crossings carry no gain information). Needs a plugin with
/// no delay between input and output.
pub fn gain_reduction_trace(x: &[f32], y: &[f32], min_abs: f32) -> Vec<(usize, f64)> {
    x.iter().zip(y).enumerate().filter(|(_, (a, _))| a.abs() >= min_abs).map(|(i, (a, b))| (i, -db(f64::from(*b) / f64::from(*a)))).collect()
}
