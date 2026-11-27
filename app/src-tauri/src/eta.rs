//! ETA estimators for the download phase of a DCS_updater run.
//!
//! Every estimator is *causal*: it sees one observation at a time (what the updater window
//! showed at time t) and answers "how many seconds until the download phase ends?".
//! The same code runs live in the monitor and offline in the replay tool
//! (`cargo run --release --example eta_replay`), which scores estimators against recorded runs.
//! See docs/ETA.md.

use std::collections::VecDeque;

/// Tail after the download phase (install + cleanup). Measured 0.7-1.6 s over 5 runs
/// (ETA.md); earlier 0.6-3.4 s (INVESTIGATION.md).
pub const INSTALL_SECONDS: f64 = 1.5;
/// What 0.1 used for the tail (kept so the v0.1 estimator replays exactly).
pub const V01_INSTALL_SECONDS: f64 = 3.0;
/// Unpacking finishes about 1 s after the last compressed byte arrives (ETA.md, last 10 %).
pub const END_LAG_SECONDS: f64 = 1.0;
/// Calibrated range around Hangar's ETA: [r × LOW − PAD, r × HIGH + PAD]. Held reality in
/// ~88 % of ticks over 5 runs; skewed upwards because network dips only ever make it later.
pub const RANGE_LOW: f64 = 0.97;
pub const RANGE_HIGH: f64 = 1.2;
pub const RANGE_PAD: f64 = 1.5;

/// One reading of the updater window.
#[derive(Clone, Debug, Default)]
pub struct Obs {
    /// Seconds since the run was first seen.
    pub t: f64,
    /// Compressed (network) progress: (done MB, total MB).
    pub comp: Option<(f64, f64)>,
    /// Unpacked progress: (done MB, total MB).
    pub unp: Option<(f64, f64)>,
    /// ED's own speed figure (compressed MB/s).
    pub ed_speed: Option<f64>,
    /// ED's own ETA in seconds.
    pub ed_eta: Option<f64>,
}

/// Seconds until the download phase ends, with an optional likely range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pred {
    pub remaining: f64,
    pub low: Option<f64>,
    pub high: Option<f64>,
}

impl Pred {
    pub fn point(remaining: f64) -> Self {
        Pred { remaining: remaining.max(0.0), low: None, high: None }
    }
}

pub trait Estimator {
    fn name(&self) -> String;
    fn observe(&mut self, o: &Obs) -> Option<Pred>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// Compressed bytes (the "Downloaded" line). Network is usually the bottleneck.
    Comp,
    /// Unpacked bytes (the "Unpacking" line).
    Unp,
}

impl Signal {
    fn get(self, o: &Obs) -> Option<(f64, f64)> {
        match self {
            Signal::Comp => o.comp,
            Signal::Unp => o.unp,
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Signal::Comp => "comp",
            Signal::Unp => "unp",
        }
    }
}

/// Minimum seconds of progress before any estimator gives a number.
const MIN_AGE: f64 = 3.0;
const MIN_RATE: f64 = 0.05;

fn remaining_at(rate: f64, done: f64, total: f64) -> Option<f64> {
    if rate < MIN_RATE {
        return None;
    }
    Some(((total - done).max(0.0)) / rate)
}

// ------------------------------------------------------------------ 0.1 (the live algorithm until now)

pub struct Estimate {
    pub eta: f64,
    pub low: f64,
    pub high: f64,
    pub rate: f64,
    pub confidence: &'static str,
}

/// Hangar 0.1: ETA from unpacked progress samples (t seconds, MB). Trimmed mean of the
/// per-tick rates in the last 60 samples, plus V01_INSTALL_SECONDS.
pub fn estimate(samples: &VecDeque<(f64, f64)>, total_mb: f64) -> Option<Estimate> {
    if samples.len() < 8 {
        return None; // too early for an honest number
    }
    let pts: Vec<&(f64, f64)> = samples.iter().collect();
    let mut rates: Vec<f64> = pts
        .windows(2)
        .filter_map(|w| {
            let dt = w[1].0 - w[0].0;
            if dt > 0.2 { Some(((w[1].1 - w[0].1) / dt).max(0.0)) } else { None }
        })
        .collect();
    if rates.len() < 5 {
        return None;
    }
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let drop = rates.len() / 10; // trim the lowest 10 % (short stalls)
    let kept = &rates[drop..];
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    if mean < MIN_RATE {
        return None;
    }
    let sd = (kept.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / kept.len() as f64).sqrt();
    let done = pts.last().map(|p| p.1).unwrap_or(0.0);
    let remaining = (total_mb - done).max(0.0);
    let cv = sd / mean;
    Some(Estimate {
        eta: remaining / mean + V01_INSTALL_SECONDS,
        low: remaining / (mean + sd) + V01_INSTALL_SECONDS * 0.5,
        high: remaining / (mean - sd).max(mean * 0.3) + V01_INSTALL_SECONDS * 1.5,
        rate: mean,
        confidence: if cv < 0.12 && kept.len() >= 30 { "high" } else if cv < 0.25 { "medium" } else { "low" },
    })
}

/// The 0.1 algorithm as an estimator (download phase only, so without V01_INSTALL_SECONDS).
#[derive(Default)]
pub struct Current {
    samples: VecDeque<(f64, f64)>,
}

impl Estimator for Current {
    fn name(&self) -> String {
        "v0.1 (trimmed mean 60 s, unpacked)".into()
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (done, total) = o.unp?;
        self.samples.push_back((o.t, done));
        while self.samples.len() > 61 {
            self.samples.pop_front();
        }
        let e = estimate(&self.samples, total)?;
        Some(Pred {
            remaining: (e.eta - V01_INSTALL_SECONDS).max(0.0),
            low: Some((e.low - V01_INSTALL_SECONDS * 0.5).max(0.0)),
            high: Some((e.high - V01_INSTALL_SECONDS * 1.5).max(0.0)),
        })
    }
}

// ------------------------------------------------------------------ ED's own figure

#[derive(Default)]
pub struct Ed;

impl Estimator for Ed {
    fn name(&self) -> String {
        "ED (window)".into()
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        o.ed_eta.map(Pred::point)
    }
}

// ------------------------------------------------------------------ simple rate models

/// Average rate since progress started (optionally skipping a warm-up, e.g. TCP ramp-up).
pub struct Average {
    pub signal: Signal,
    pub warmup: f64,
    first: Option<f64>,
    origin: Option<(f64, f64)>,
}

impl Average {
    pub fn new(signal: Signal, warmup: f64) -> Self {
        Average { signal, warmup, first: None, origin: None }
    }
}

impl Estimator for Average {
    fn name(&self) -> String {
        format!("average {} (skip {} s)", self.signal.tag(), self.warmup)
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (done, total) = self.signal.get(o)?;
        let first = *self.first.get_or_insert(o.t);
        if o.t - first < self.warmup {
            return None;
        }
        let (t0, x0) = *self.origin.get_or_insert((o.t, done));
        if o.t - t0 < MIN_AGE {
            return None;
        }
        remaining_at((done - x0) / (o.t - t0), done, total).map(Pred::point)
    }
}

/// Mean rate over the last `window` seconds.
pub struct Window {
    pub signal: Signal,
    pub window: f64,
    pts: VecDeque<(f64, f64)>,
}

impl Window {
    pub fn new(signal: Signal, window: f64) -> Self {
        Window { signal, window, pts: VecDeque::new() }
    }
}

impl Estimator for Window {
    fn name(&self) -> String {
        format!("window {} {} s", self.signal.tag(), self.window)
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (done, total) = self.signal.get(o)?;
        self.pts.push_back((o.t, done));
        while self.pts.len() > 2 && o.t - self.pts[1].0 >= self.window {
            self.pts.pop_front();
        }
        let (t0, x0) = *self.pts.front()?;
        if o.t - t0 < MIN_AGE {
            return None;
        }
        remaining_at((done - x0) / (o.t - t0), done, total).map(Pred::point)
    }
}

/// Exponentially weighted moving average of the rate, time-based (works with uneven ticks)
/// and seeded with the first measured rate (no bias from starting at zero).
pub struct Ewma {
    pub signal: Signal,
    pub tau: f64,
    first_t: Option<f64>,
    last: Option<(f64, f64)>,
    rate: Option<f64>,
}

impl Ewma {
    pub fn new(signal: Signal, tau: f64) -> Self {
        Ewma { signal, tau, first_t: None, last: None, rate: None }
    }
}

impl Estimator for Ewma {
    fn name(&self) -> String {
        format!("EWMA {} tau {} s", self.signal.tag(), self.tau)
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (done, total) = self.signal.get(o)?;
        let first = *self.first_t.get_or_insert(o.t);
        if let Some((lt, lx)) = self.last {
            let dt = o.t - lt;
            if dt > 0.05 {
                let inst = ((done - lx) / dt).max(0.0);
                let a = 1.0 - (-dt / self.tau).exp();
                self.rate = Some(match self.rate {
                    None => inst,
                    Some(r) => r + a * (inst - r),
                });
                self.last = Some((o.t, done));
            }
        } else {
            self.last = Some((o.t, done));
        }
        if o.t - first < MIN_AGE {
            return None;
        }
        remaining_at(self.rate?, done, total).map(Pred::point)
    }
}

/// Compressed remaining divided by ED's own speed figure (optionally smoothed).
pub struct EdSpeed {
    pub tau: f64,
    first_t: Option<f64>,
    last_t: Option<f64>,
    speed: Option<f64>,
}

impl EdSpeed {
    pub fn new(tau: f64) -> Self {
        EdSpeed { tau, first_t: None, last_t: None, speed: None }
    }
}

impl Estimator for EdSpeed {
    fn name(&self) -> String {
        if self.tau <= 0.0 { "ED speed, comp".into() } else { format!("ED speed EWMA {} s, comp", self.tau) }
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (done, total) = o.comp?;
        let sp = o.ed_speed?;
        let first = *self.first_t.get_or_insert(o.t);
        let dt = self.last_t.map(|l| o.t - l).unwrap_or(0.0);
        self.last_t = Some(o.t);
        self.speed = Some(match (self.speed, self.tau > 0.0) {
            (Some(s), true) => s + (1.0 - (-dt / self.tau).exp()) * (sp - s),
            _ => sp,
        });
        if o.t - first < MIN_AGE {
            return None;
        }
        remaining_at(self.speed?, done, total).map(Pred::point)
    }
}

// ------------------------------------------------------------------ Kalman filter

/// Constant-velocity Kalman filter on progress (state: MB done, MB/s). `q` is the variance
/// growth of the rate per second (how fast the true speed may wander), `r` the measurement
/// variance (MB²). Gives an 80 % range from the rate's uncertainty.
pub struct Kalman {
    pub signal: Signal,
    pub q: f64,
    pub r: f64,
    first_t: Option<f64>,
    last_t: f64,
    x: [f64; 2],
    p: [[f64; 2]; 2],
    init: u8,
    x0: (f64, f64),
}

impl Kalman {
    pub fn new(signal: Signal, q: f64, r: f64) -> Self {
        Kalman { signal, q, r, first_t: None, last_t: 0.0, x: [0.0; 2], p: [[0.0; 2]; 2], init: 0, x0: (0.0, 0.0) }
    }
}

impl Estimator for Kalman {
    fn name(&self) -> String {
        format!("Kalman {} q {} r {}", self.signal.tag(), self.q, self.r)
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let (z, total) = self.signal.get(o)?;
        let first = *self.first_t.get_or_insert(o.t);
        match self.init {
            0 => {
                self.x0 = (o.t, z);
                self.init = 1;
                self.last_t = o.t;
                return None;
            }
            1 => {
                let dt = o.t - self.x0.0;
                if dt < 0.5 {
                    return None;
                }
                let v = (z - self.x0.1) / dt;
                self.x = [z, v];
                self.p = [[self.r, 0.0], [0.0, (v * 0.5).powi(2).max(1.0)]];
                self.init = 2;
                self.last_t = o.t;
            }
            _ => {
                let dt = (o.t - self.last_t).max(0.0);
                self.last_t = o.t;
                // predict
                let [x, v] = self.x;
                self.x = [x + v * dt, v];
                let p = self.p;
                let q = self.q;
                let p00 = p[0][0] + dt * (p[1][0] + p[0][1]) + dt * dt * p[1][1] + q * dt.powi(3) / 3.0;
                let p01 = p[0][1] + dt * p[1][1] + q * dt * dt / 2.0;
                let p10 = p[1][0] + dt * p[1][1] + q * dt * dt / 2.0;
                let p11 = p[1][1] + q * dt;
                // update with measurement z of position
                let s = p00 + self.r;
                let k0 = p00 / s;
                let k1 = p10 / s;
                let y = z - self.x[0];
                self.x = [self.x[0] + k0 * y, (self.x[1] + k1 * y).max(0.0)];
                self.p = [[(1.0 - k0) * p00, (1.0 - k0) * p01], [p10 - k1 * p00, p11 - k1 * p01]];
            }
        }
        if o.t - first < MIN_AGE {
            return None;
        }
        let v = self.x[1];
        let rem_mb = (total - z).max(0.0);
        let rem = remaining_at(v, z, total)?;
        let sd = self.p[1][1].max(0.0).sqrt();
        Some(Pred {
            remaining: rem,
            low: Some(rem_mb / (v + 1.2816 * sd)),
            high: Some(rem_mb / (v - 1.2816 * sd).max(v * 0.3)),
        })
    }
}

// ------------------------------------------------------------------ combinations

/// Smooths the predicted *finish time* (t + remaining) so the shown clock does not jump.
pub struct Smoothed<E: Estimator> {
    pub inner: E,
    pub tau: f64,
    finish: Option<(f64, f64)>,
}

impl<E: Estimator> Smoothed<E> {
    pub fn new(inner: E, tau: f64) -> Self {
        Smoothed { inner, tau, finish: None }
    }
}

impl<E: Estimator> Estimator for Smoothed<E> {
    fn name(&self) -> String {
        format!("{} + finish smoothing {} s", self.inner.name(), self.tau)
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        let p = self.inner.observe(o)?;
        let raw = o.t + p.remaining;
        let f = match self.finish {
            None => raw,
            Some((lt, lf)) => lf + (1.0 - (-(o.t - lt).max(0.0) / self.tau).exp()) * (raw - lf),
        };
        self.finish = Some((o.t, f));
        let shift = f - raw;
        Some(Pred {
            remaining: (f - o.t).max(0.0),
            low: p.low.map(|l| (l + shift).max(0.0)),
            high: p.high.map(|h| (h + shift).max(0.0)),
        })
    }
}

/// The later of a compressed-based and an unpacked-based estimate: the phase ends when both
/// the download and the unpacking are done.
pub struct MaxOf<A: Estimator, B: Estimator> {
    pub a: A,
    pub b: B,
}

impl<A: Estimator, B: Estimator> Estimator for MaxOf<A, B> {
    fn name(&self) -> String {
        format!("max({}, {})", self.a.name(), self.b.name())
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        match (self.a.observe(o), self.b.observe(o)) {
            (Some(x), Some(y)) => Some(if x.remaining >= y.remaining { x } else { y }),
            (x, y) => x.or(y),
        }
    }
}

// ------------------------------------------------------------------ network graph

/// The updater's own download rate for the network graph: median of the last 5 per-tick rates
/// of the compressed figure. ED's counter jumps ~30 MB in a single tick about once a minute
/// without matching traffic (recording 2026-11-27); a median hides such single steps while a
/// real dip still shows within 2-3 ticks.
#[derive(Default)]
pub struct NetRate {
    last: Option<(f64, f64)>,
    rates: VecDeque<f64>,
}

impl NetRate {
    pub fn push(&mut self, t: f64, done_mb: f64) -> Option<f64> {
        if let Some((lt, ld)) = self.last {
            let dt = t - lt;
            if dt < 0.2 {
                return self.median();
            }
            self.rates.push_back(((done_mb - ld) / dt).max(0.0));
            while self.rates.len() > 5 {
                self.rates.pop_front();
            }
        }
        self.last = Some((t, done_mb));
        self.median()
    }

    fn median(&self) -> Option<f64> {
        if self.rates.is_empty() {
            return None;
        }
        let mut v: Vec<f64> = self.rates.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(v[v.len() / 2])
    }
}

// ------------------------------------------------------------------ the chosen algorithm (0.2)

/// Hangar's ETA from 0.2 on, chosen with the replay (docs/ETA.md):
/// mean compressed rate over the last 120 s, plus END_LAG_SECONDS for unpacking to catch up,
/// with a calibrated range. Without the compressed figure (window not readable) it falls back
/// to unpacked progress (from the window or `_downloads`) with low confidence.
pub struct HangarEta {
    comp: Window,
    unp: Window,
    short: Ewma,
    started: Option<f64>,
    comp_seen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HangarAnswer {
    pub pred: Pred,
    /// "high" | "medium" | "low"
    pub confidence: &'static str,
    /// Rate behind the answer (MB/s, compressed or unpacked).
    pub rate: f64,
}

impl Default for HangarEta {
    fn default() -> Self {
        HangarEta { comp: Window::new(Signal::Comp, 120.0), unp: Window::new(Signal::Unp, 120.0), short: Ewma::new(Signal::Comp, 10.0), started: None, comp_seen: false }
    }
}

fn ranged(remaining: f64) -> Pred {
    Pred {
        remaining,
        low: Some((remaining * RANGE_LOW - RANGE_PAD).max(0.0)),
        high: Some(remaining * RANGE_HIGH + RANGE_PAD),
    }
}

impl HangarEta {
    pub fn answer(&mut self, o: &Obs) -> Option<HangarAnswer> {
        let started = *self.started.get_or_insert(o.t);
        let comp = self.comp.observe(o);
        let short = self.short.observe(o);
        let unp = self.unp.observe(o);
        if let (Some(p), Some((done, total))) = (comp, o.comp) {
            let rem_mb = (total - done).max(0.0);
            let rate = if p.remaining > 0.0 { rem_mb / p.remaining } else { 0.0 };
            // a dip right now (short-term rate well below the 120 s mean) or a young estimate
            let dipping = short.is_some_and(|s| s.remaining > p.remaining * 1.25);
            let confidence = if o.t - started < 15.0 || dipping { "medium" } else { "high" };
            let remaining = if rem_mb > 0.0 { p.remaining + END_LAG_SECONDS } else { END_LAG_SECONDS * 0.5 };
            return Some(HangarAnswer { pred: ranged(remaining), confidence, rate });
        }
        // Fallback only when the compressed figure never shows up: the window can show
        // "Unpacking" a tick before "Downloaded" (recording 2026-11-27).
        if o.comp.is_some() {
            self.comp_seen = true;
        }
        if self.comp_seen || o.t - started < 10.0 {
            return None;
        }
        let p = unp?;
        let (done, total) = o.unp?;
        let rate = if p.remaining > 0.0 { (total - done).max(0.0) / p.remaining } else { 0.0 };
        // unpacked progress is uneven (ETA.md): wide range, low confidence
        Some(HangarAnswer {
            pred: Pred { remaining: p.remaining, low: Some(p.remaining * 0.6), high: Some(p.remaining * 1.8 + RANGE_PAD) },
            confidence: "low",
            rate,
        })
    }
}

impl Estimator for HangarEta {
    fn name(&self) -> String {
        "v0.2 Hangar (window comp 120 s + lag, range)".into()
    }
    fn observe(&mut self, o: &Obs) -> Option<Pred> {
        self.answer(o).map(|a| a.pred)
    }
}

/// Every candidate the replay tool scores. Order = order in the report.
pub fn candidates() -> Vec<Box<dyn Estimator>> {
    use Signal::*;
    let mut v: Vec<Box<dyn Estimator>> = vec![Box::new(HangarEta::default()), Box::new(Current::default()), Box::new(Ed)];
    for s in [Comp, Unp] {
        v.push(Box::new(Average::new(s, 0.0)));
        v.push(Box::new(Average::new(s, 10.0)));
        for w in [15.0, 30.0, 60.0, 120.0] {
            v.push(Box::new(Window::new(s, w)));
        }
        for tau in [5.0, 15.0, 30.0, 60.0] {
            v.push(Box::new(Ewma::new(s, tau)));
        }
        for q in [0.05, 0.5, 5.0] {
            v.push(Box::new(Kalman::new(s, q, 4.0)));
        }
    }
    // longer memory (round 2, after the first real recordings)
    v.push(Box::new(Window::new(Comp, 180.0)));
    v.push(Box::new(Window::new(Comp, 300.0)));
    v.push(Box::new(Ewma::new(Comp, 90.0)));
    v.push(Box::new(Ewma::new(Comp, 120.0)));
    // long memory, but a dip shows at once (the slower of long and short)
    v.push(Box::new(MaxOf { a: Window::new(Comp, 120.0), b: Ewma::new(Comp, 10.0) }));
    v.push(Box::new(MaxOf { a: Window::new(Comp, 120.0), b: Window::new(Comp, 30.0) }));
    v.push(Box::new(MaxOf { a: Ewma::new(Comp, 60.0), b: Ewma::new(Comp, 15.0) }));
    v.push(Box::new(EdSpeed::new(0.0)));
    v.push(Box::new(EdSpeed::new(15.0)));
    v.push(Box::new(MaxOf { a: Ewma::new(Comp, 30.0), b: Ewma::new(Unp, 30.0) }));
    for tau in [5.0, 15.0] {
        v.push(Box::new(Smoothed::new(Ewma::new(Comp, 15.0), tau)));
        v.push(Box::new(Smoothed::new(Kalman::new(Comp, 0.5, 4.0), tau)));
        v.push(Box::new(Smoothed::new(Window::new(Comp, 30.0), tau)));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(t: f64, comp: f64, unp: f64) -> Obs {
        Obs { t, comp: Some((comp, 1000.0)), unp: Some((unp, 1500.0)), ed_speed: Some(20.0), ed_eta: Some(10.0) }
    }

    #[test]
    fn current_matches_old_formula() {
        let mut s = VecDeque::new();
        for i in 0..40 {
            s.push_back((i as f64, i as f64 * 46.0));
        }
        let e = estimate(&s, 1892.7).unwrap();
        assert!((e.eta - (98.7 / 46.0 + V01_INSTALL_SECONDS)).abs() < 0.1);
        assert_eq!(e.confidence, "high");
        let few: VecDeque<(f64, f64)> = s.iter().take(4).copied().collect();
        assert!(estimate(&few, 1892.7).is_none());
    }

    #[test]
    fn every_candidate_is_exact_on_a_constant_rate() {
        // 20 MB/s compressed, 30 MB/s unpacked; both finish at t = 50 s.
        for mut e in candidates() {
            let mut last = None;
            for i in 0..=40 {
                let t = i as f64;
                last = e.observe(&obs(t, 20.0 * t, 30.0 * t)).or(last);
            }
            let p = last.unwrap_or_else(|| panic!("{} gave nothing", e.name()));
            if e.name().starts_with("ED") && !e.name().contains("speed") {
                continue; // fixed fake ED value in this test
            }
            let expect = if e.name().starts_with("v0.2") { 10.0 + END_LAG_SECONDS } else { 10.0 };
            assert!((p.remaining - expect).abs() < 0.6, "{}: {}", e.name(), p.remaining);
        }
    }

    #[test]
    fn hangar_fallback_waits_for_the_compressed_figure() {
        // first tick has only "Unpacking" (as recorded), then both lines
        let mut e = HangarEta::default();
        let first = Obs { t: 0.0, unp: Some((0.0, 1500.0)), ..Default::default() };
        assert!(e.answer(&first).is_none());
        for i in 1..=12 {
            let t = i as f64;
            let a = e.answer(&obs(t, 20.0 * t, 5.0 * t));
            if let Some(a) = a {
                assert_ne!(a.confidence, "low", "unpacked fallback used at t={t}");
            }
        }
        // window without "Downloaded" at all: low-confidence answer only after 10 s
        let mut e = HangarEta::default();
        let only_unp = |t: f64| Obs { t, unp: Some((30.0 * t, 1500.0)), ..Default::default() };
        assert!(e.answer(&only_unp(0.0)).is_none());
        assert!(e.answer(&only_unp(5.0)).is_none());
        assert_eq!(e.answer(&only_unp(11.0)).unwrap().confidence, "low");
    }

    #[test]
    fn net_rate_hides_single_steps_but_shows_dips() {
        let mut n = NetRate::default();
        let mut done = 0.0;
        let mut out = Vec::new();
        for i in 0..30 {
            let t = i as f64;
            done += match i {
                10 => 32.0 + 31.0, // ED's step (recorded: 1651.6 -> 1714.7 MB in 1 s)
                20..=25 => 8.0,    // a real dip
                _ => 32.0,
            };
            out.push(n.push(t, done));
        }
        assert!(out[10].unwrap() < 33.0, "step leaked: {:?}", out[10]);
        assert!(out[11].unwrap() < 33.0);
        assert!(out[23].unwrap() < 9.0, "dip hidden: {:?}", out[23]);
        assert!((out[29].unwrap() - 32.0).abs() < 0.1);
    }

    #[test]
    fn nothing_before_min_age() {
        let mut e = Ewma::new(Signal::Comp, 15.0);
        assert!(e.observe(&obs(0.0, 0.0, 0.0)).is_none());
        assert!(e.observe(&obs(1.0, 20.0, 30.0)).is_none());
        assert!(e.observe(&obs(4.0, 80.0, 120.0)).is_some());
    }
}
