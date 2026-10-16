//! Forked from upstream `crates/dsp/src/plugins/eq.rs` at commit c73a8e2 (storytold/soundcraft).
//!
//! WA EQ 7 (`wa_eq7`): HPF, low shelf, three peaks, high shelf, LPF, as RBJ cookbook biquads in
//! series, with smoothed parameters and input and output gain. [`wa_eq7_response`] is the
//! analytic magnitude response the measurement tests compare the running filter with.

use soundcraft_dsp::biquad::{Biquad, Coeffs, FilterType, butterworth_qs};
use soundcraft_dsp::params::{Params, choice, lin, log, toggle};
use soundcraft_dsp::util::{Smoother, frames_in, sane_channels, sane_sr};
use soundcraft_dsp::{Category, ParamInfo, Plugin, PluginInfo, Unit, db_to_gain};

const SLOPES: &[&str] = &["6 dB/oct", "12 dB/oct", "18 dB/oct", "24 dB/oct"];

static WA_EQ7_PARAMS: [ParamInfo; 23] = [
    lin("input_gain", "Input", -24.0, 24.0, 0.0, Unit::Db),
    toggle("hpf_on", "HPF", false),
    log("hpf_freq", "HPF Freq", 10.0, 2000.0, 80.0, Unit::Hz),
    choice("hpf_slope", "HPF Slope", SLOPES, 1),
    log("low_shelf_freq", "LF Freq", 20.0, 1000.0, 100.0, Unit::Hz),
    lin("low_shelf_gain", "LF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("low_shelf_q", "LF Q", 0.3, 2.0, 0.707, Unit::None),
    log("low_mid_freq", "LMF Freq", 20.0, 2000.0, 250.0, Unit::Hz),
    lin("low_mid_gain", "LMF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("low_mid_q", "LMF Q", 0.1, 10.0, 1.0, Unit::None),
    log("mid_freq", "MF Freq", 100.0, 8000.0, 1000.0, Unit::Hz),
    lin("mid_gain", "MF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("mid_q", "MF Q", 0.1, 10.0, 1.0, Unit::None),
    log("high_mid_freq", "HMF Freq", 500.0, 20000.0, 4000.0, Unit::Hz),
    lin("high_mid_gain", "HMF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("high_mid_q", "HMF Q", 0.1, 10.0, 1.0, Unit::None),
    log("high_shelf_freq", "HF Freq", 1000.0, 20000.0, 8000.0, Unit::Hz),
    lin("high_shelf_gain", "HF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("high_shelf_q", "HF Q", 0.3, 2.0, 0.707, Unit::None),
    toggle("lpf_on", "LPF", false),
    log("lpf_freq", "LPF Freq", 1000.0, 20000.0, 18000.0, Unit::Hz),
    choice("lpf_slope", "LPF Slope", SLOPES, 1),
    lin("output_gain", "Output", -24.0, 24.0, 0.0, Unit::Db),
];

pub static WA_EQ7_INFO: PluginInfo = PluginInfo {
    id: "wa_eq7",
    name: "WA EQ 7",
    short_name: "WA EQ7",
    category: Category::Eq,
    params: &WA_EQ7_PARAMS,
    is_instrument: false,
    audiosuite: true,
};

/// Indices of the input and output gain parameters, summed into the broadband gain.
const GAIN_IDS: [usize; 2] = [0, 22];
/// Biquad sections: HPF (2), five bands, LPF (2).
const SECTIONS: usize = 9;

#[inline]
fn val(v: &[f32], i: usize) -> f32 {
    v.get(i).copied().unwrap_or(0.0)
}

/// 1–2 cascaded sections for an HPF/LPF of the given slope index (6/12/18/24 dB/oct),
/// Butterworth in every slope.
fn pass_filter(slope: usize, high: bool, freq: f32, sr: f32) -> [Coeffs; 2] {
    let (k1, k2) = if high { (FilterType::HighPass1, FilterType::HighPass) } else { (FilterType::LowPass1, FilterType::LowPass) };
    match slope {
        0 => [Coeffs::design(k1, freq, 0.707, 0.0, sr), Coeffs::IDENTITY],
        2 => [Coeffs::design(k1, freq, 0.707, 0.0, sr), Coeffs::design(k2, freq, 1.0, 0.0, sr)],
        3 => {
            let q = butterworth_qs(4);
            let q0 = q.first().copied().unwrap_or(0.54);
            let q1 = q.get(1).copied().unwrap_or(1.31);
            [Coeffs::design(k2, freq, q0, 0.0, sr), Coeffs::design(k2, freq, q1, 0.0, sr)]
        }
        _ => [Coeffs::design(k2, freq, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr), Coeffs::IDENTITY],
    }
}

fn design(v: &[f32], sr: f32) -> [Coeffs; SECTIONS] {
    let mut c = [Coeffs::IDENTITY; SECTIONS];
    if val(v, 1) >= 0.5 {
        let [a, b] = pass_filter(val(v, 3).round() as usize, true, val(v, 2), sr);
        c[0] = a;
        c[1] = b;
    }
    let bands = [(4, FilterType::LowShelf), (7, FilterType::Peak), (10, FilterType::Peak), (13, FilterType::Peak), (16, FilterType::HighShelf)];
    for (k, (i, kind)) in bands.into_iter().enumerate() {
        let g = val(v, i + 1);
        if g.abs() > 1e-4
            && let Some(slot) = c.get_mut(2 + k)
        {
            *slot = Coeffs::design(kind, val(v, i), val(v, i + 2), g, sr);
        }
    }
    if val(v, 19) >= 0.5 {
        let [a, b] = pass_filter(val(v, 21).round() as usize, false, val(v, 20), sr);
        c[7] = a;
        c[8] = b;
    }
    c
}

/// WA EQ 7: nine smoothed biquad sections per channel and a smoothed broadband gain.
pub struct WaEq7 {
    p: Params,
    sm: Vec<Smoother>,
    vals: Vec<f32>,
    coeffs: [Coeffs; SECTIONS],
    filt: Vec<[Biquad; SECTIONS]>,
    gain: Smoother,
    sr: f32,
}

impl Default for WaEq7 {
    fn default() -> Self {
        Self::new()
    }
}

impl WaEq7 {
    pub fn new() -> Self {
        let params = WA_EQ7_INFO.params;
        let mut eq = Self {
            p: Params::new(params),
            sm: params.iter().map(|i| Smoother::new(i.default)).collect(),
            vals: params.iter().map(|i| i.default).collect(),
            coeffs: [Coeffs::IDENTITY; SECTIONS],
            filt: Vec::new(),
            gain: Smoother::new(1.0),
            sr: 48_000.0,
        };
        eq.redesign();
        eq
    }

    fn total_gain_db(&self, v: impl Fn(usize) -> f32) -> f32 {
        GAIN_IDS.iter().map(|&i| v(i)).sum()
    }

    fn update(&mut self) {
        let mut snapped = false;
        for (i, (s, info)) in self.sm.iter_mut().zip(WA_EQ7_INFO.params).enumerate() {
            s.set(self.p.v(i));
            if matches!(info.unit, Unit::Toggle | Unit::Choice) {
                s.snap();
                snapped |= self.vals.get(i) != Some(&s.value());
            }
        }
        // A snapped smoother never settles, so `process` would not redesign for it.
        if snapped {
            self.redesign();
        }
        let g = db_to_gain(self.total_gain_db(|i| self.p.v(i)));
        self.gain.set(g);
    }

    fn redesign(&mut self) {
        for (v, s) in self.vals.iter_mut().zip(&self.sm) {
            *v = s.value();
        }
        self.coeffs = design(&self.vals, self.sr);
        for f in &mut self.filt {
            for (b, c) in f.iter_mut().zip(&self.coeffs) {
                b.coeffs = *c;
            }
        }
    }

    /// Magnitude response (dB, including input and output gain) of the current settings.
    pub fn response_db(&self, freqs: &[f32], sample_rate: f32) -> Vec<f32> {
        let vals: Vec<f32> = (0..WA_EQ7_INFO.params.len()).map(|i| self.p.v(i)).collect();
        response(&vals, freqs, sample_rate)
    }
}

fn response(vals: &[f32], freqs: &[f32], sr: f32) -> Vec<f32> {
    let coeffs = design(vals, sr);
    let g: f32 = GAIN_IDS.iter().map(|&i| val(vals, i)).sum();
    freqs.iter().map(|&f| g + coeffs.iter().map(|c| c.magnitude_db(f, sr)).sum::<f32>()).collect()
}

/// WA EQ 7 magnitude response in dB at `freqs`, from the same (f32) coefficients the running
/// filter uses. `params` overrides the defaults by id; an unknown id is an error.
pub fn wa_eq7_response(params: &[(&str, f32)], freqs: &[f32], sr: f32) -> Result<Vec<f32>, String> {
    let mut p = Params::new(WA_EQ7_INFO.params);
    for (id, v) in params {
        p.set(id, *v).ok_or_else(|| format!("wa_eq7 has no parameter {id:?}"))?;
    }
    let vals: Vec<f32> = (0..WA_EQ7_INFO.params.len()).map(|i| p.v(i)).collect();
    Ok(response(&vals, freqs, sr))
}

impl Plugin for WaEq7 {
    fn info(&self) -> &'static PluginInfo {
        &WA_EQ7_INFO
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if self.p.set(id, value).is_some() {
            self.update();
            true
        } else {
            false
        }
    }

    fn param(&self, id: &str) -> Option<f32> {
        self.p.get(id)
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sane_sr(sample_rate);
        for s in &mut self.sm {
            s.set_time(20.0, self.sr);
        }
        self.gain.set_time(20.0, self.sr);
        self.filt = vec![[Biquad::default(); SECTIONS]; sane_channels(channels)];
        self.update();
        self.sm.iter_mut().for_each(Smoother::snap);
        self.gain.snap();
        self.redesign();
    }

    fn reset(&mut self) {
        self.filt.iter_mut().flatten().for_each(Biquad::reset);
        self.sm.iter_mut().for_each(Smoother::snap);
        self.gain.snap();
        self.redesign();
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        const CHUNK: usize = 32;
        let frames = frames_in(io, frames, self.filt.len());
        let mut pos = 0;
        while pos < frames {
            let n = CHUNK.min(frames - pos);
            if self.sm.iter().any(Smoother::settling) {
                self.sm.iter_mut().for_each(|s| {
                    s.advance(n);
                });
                self.redesign();
            }
            let mut gains = [0.0f32; CHUNK];
            for g in gains.iter_mut().take(n) {
                *g = self.gain.next();
            }
            for (buf, f) in io.iter_mut().zip(self.filt.iter_mut()) {
                let Some(seg) = buf.get_mut(pos..pos + n) else { continue };
                for (x, g) in seg.iter_mut().zip(&gains) {
                    let mut y = *x;
                    for b in f.iter_mut() {
                        y = b.process_sample(y);
                    }
                    *x = y * g;
                }
            }
            pos += n;
        }
    }
}
