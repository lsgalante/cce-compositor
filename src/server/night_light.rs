// SPDX-License-Identifier: GPL-3.0-only

//! Night light: warm the displays toward a lower colour temperature at
//! night, through each output's hardware gamma LUT.
//!
//! The LUT is a KMS CRTC property (`GAMMA_LUT`), applied by the display
//! engine after composition: it costs no GPU time, direct scanout and
//! fullscreen games keep working, and screenshots stay neutral. It goes out
//! as the output state's colour transform
//! (`wlr_output_state_set_color_transform`), added to the frame
//! `Output::render_and_commit` is about to commit ([`prepare`], then
//! [`committed`] once the commit took). An output with no LUT
//! (`wlr_output_get_gamma_size` 0: headless, some drivers) is left alone and
//! reported as `unsupported`; fx_renderer has no colour-transform pass, so
//! there is no software fallback.
//!
//! **A gamma-control client wins.** wlr-gamma-control (gammastep, a game, a
//! calibration tool) is exclusive per output, and scenefx already commits a
//! client's ramp (`scene_output_state_attempt_gamma`). While a client holds
//! an output's control, this module does not touch that output; when the
//! client lets go, scenefx commits the identity and this module puts its own
//! LUT back in the same frame.
//!
//! The colour: the Planckian locus point for the temperature (Kim et al.'s
//! cubic fit, 1667-25000 K), converted to linear sRGB and divided by the
//! 6500 K point, so 6500 K is exactly neutral, then scaled so the largest
//! channel is 1. The LUT scales linear light by that white point:
//! `out = encode(decode(in) * white)`. That matches gammastep and redshift
//! (whose table is that white point gamma-encoded, multiplied into the
//! encoded ramp) to within ~0.025 per channel, so "3500" here looks like
//! 3500 there. Temperatures are blended in mireds (1e6 / K), the
//! perceptually even scale.
//!
//! The schedule: `night_light { }` in config.kdl. `location latitude=…
//! longitude=…` follows the sun (NOAA's solar equations); `start "21:00"` /
//! `end "07:00"` are fixed. Each changeover fades over `transition` minutes
//! centred on the boundary. `ccectl night-light on|off|auto|toggle` overrides
//! the schedule until the next config load; a change made that way, a
//! config load and a resume fade over [`FADE_MS`] (instant when animations
//! are off, `cce_core::motion`). Without a `night_light` block the module
//! never touches an output.
//!
//! The state is one per thread (`STATE`), on the compositor's main thread:
//! the frame path that asks [`prepare`] holds no other subsystem, and the
//! timer finds it by place, like the shared state (`crate::shared`).
//!
//! A frame is asked for only when the LUT has to change: the timer sleeps
//! until the next transition (at most [`IDLE_RECHECK_MS`] while a schedule
//! runs), steps every [`TRANSITION_STEP_MS`] through one, and is never armed
//! without a `night_light` block or with a fixed `on`/`off`.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use crate::ffi;

/// The temperature that changes nothing: the sRGB white point.
pub const NEUTRAL_K: u32 = 6500;
/// The range a temperature is clamped to (the locus fit holds from 1667 K).
pub const MIN_K: u32 = 1700;
pub const MAX_K: u32 = 12000;
const DEFAULT_NIGHT_K: u32 = 4000;
const DEFAULT_TRANSITION_MIN: f64 = 45.0;
/// A manual change, a config load or a resume fades over this long.
pub const FADE_MS: f64 = 1500.0;
/// Frame interval while a fade runs.
const FADE_STEP_MS: i32 = 33;
/// Through a scheduled transition (a 45-minute fade moves ~3000 K, so a
/// 10 s step is ~10 K: invisible).
const TRANSITION_STEP_MS: i32 = 10_000;
/// Outside a transition, the longest the timer sleeps before re-reading the
/// clock, so a clock change is followed within this long. A resume is
/// followed at once ([`session_active`]).
const IDLE_RECHECK_MS: i32 = 600_000;
/// A target this far (K) from what is shown is a jump, which fades; a
/// smaller one (a scheduled transition's step) is applied as it comes.
const JUMP_K: f64 = 50.0;
const DAY_MIN: f64 = 1440.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Follow the schedule (`on` when there is none).
    Auto,
    /// The night temperature, now.
    On,
    /// The day temperature, now.
    Off,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "on" => Some(Mode::On),
            "off" => Some(Mode::Off),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::On => "on",
            Mode::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Schedule {
    None,
    /// Night from `start` to `end`, minutes after local midnight.
    Fixed { start: f64, end: f64 },
    /// Night from sunset to sunrise; degrees, longitude east-positive.
    Sun { lat: f64, lon: f64 },
}

/// The `night_light { }` block.
#[derive(Debug, Clone, PartialEq)]
pub struct NightLightConfig {
    pub night_k: u32,
    pub day_k: u32,
    pub schedule: Schedule,
    pub transition_min: f64,
    pub mode: Mode,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        NightLightConfig {
            night_k: DEFAULT_NIGHT_K,
            day_k: NEUTRAL_K,
            schedule: Schedule::None,
            transition_min: DEFAULT_TRANSITION_MIN,
            mode: Mode::Auto,
        }
    }
}

pub fn clamp_k(k: i64) -> u32 {
    k.clamp(MIN_K as i64, MAX_K as i64) as u32
}

/// `"21:30"` → 1290.0 (minutes after midnight).
pub fn parse_clock(s: &str) -> Option<f64> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h * 60 + m) as f64)
}

fn fmt_clock(min: f64) -> String {
    let m = (min.rem_euclid(DAY_MIN)).round() as u32 % 1440;
    format!("{:02}:{:02}", m / 60, m % 60)
}

// ---------------------------------------------------------------------------
// Colour

/// The CIE 1931 xy chromaticity of a black body at `k` kelvin (Kim et al.,
/// the Planckian locus cubic spline; 1667-25000 K).
fn planck_xy(k: f64) -> (f64, f64) {
    let t = k.clamp(1667.0, 25000.0);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    (x, y)
}

/// Linear sRGB (D65) of chromaticity (x, y) at luminance 1.
fn xy_to_linear_srgb((x, y): (f64, f64)) -> [f64; 3] {
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    [
        3.2404542 * cx - 1.5371385 - 0.4985314 * cz,
        -0.9692660 * cx + 1.8760108 + 0.0415560 * cz,
        0.0556434 * cx - 0.2040259 + 1.0572252 * cz,
    ]
}

/// The linear-light multipliers that move white to `k` kelvin: 6500 K is
/// `[1, 1, 1]`, and the largest channel is always 1.
pub fn white_point(k: f64) -> [f64; 3] {
    if (k - NEUTRAL_K as f64).abs() < 0.5 {
        return [1.0; 3];
    }
    let rgb = xy_to_linear_srgb(planck_xy(k));
    let neutral = xy_to_linear_srgb(planck_xy(NEUTRAL_K as f64));
    let rel = [rgb[0] / neutral[0], rgb[1] / neutral[1], rgb[2] / neutral[2]].map(|c| c.max(0.0));
    let max = rel.iter().cloned().fold(f64::MIN, f64::max);
    rel.map(|c| c / max)
}

fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(v: f64) -> f64 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// One channel of a `size`-entry gamma ramp that scales linear light by
/// `gain`.
pub fn ramp_channel(size: usize, gain: f64) -> Vec<u16> {
    let last = size.saturating_sub(1).max(1) as f64;
    (0..size)
        .map(|i| {
            let v = srgb_encode(srgb_decode(i as f64 / last) * gain);
            (v.clamp(0.0, 1.0) * 65535.0).round() as u16
        })
        .collect()
}

fn to_mired(k: f64) -> f64 {
    1e6 / k
}

/// The temperature `f` of the way from `day` to `night` (0 day, 1 night),
/// blended in mireds.
pub fn blend_k(day: f64, night: f64, f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    1e6 / (to_mired(day) + (to_mired(night) - to_mired(day)) * f)
}

// ---------------------------------------------------------------------------
// Schedule

/// How far into the night `t` is, 0 (day) to 1 (night), for a night from
/// `start` to `end` (minutes after midnight, wrapping) whose changeovers fade
/// over `transition` minutes centred on each boundary.
pub fn night_factor(t: f64, start: f64, end: f64, transition: f64) -> f64 {
    let len = (end - start).rem_euclid(DAY_MIN);
    if len <= 0.0 {
        return 0.0;
    }
    let d = transition.clamp(0.0, len.min(DAY_MIN - len)).max(1e-6);
    let ramp = |x: f64| (0.5 + x / d).clamp(0.0, 1.0);
    let u = (t - start).rem_euclid(DAY_MIN);
    if u < len {
        ramp(u).min(ramp(len - u))
    } else {
        ramp(-(u - len)).max(ramp(-(DAY_MIN - u)))
    }
}

/// Milliseconds until the night factor next starts to move (the earlier
/// boundary's fade start), or `None` while it is moving.
fn ms_until_transition(t: f64, start: f64, end: f64, transition: f64) -> Option<f64> {
    let f = night_factor(t, start, end, transition);
    if f > 0.0 && f < 1.0 {
        return None;
    }
    let len = (end - start).rem_euclid(DAY_MIN);
    let half = transition.clamp(0.0, len.min(DAY_MIN - len)) / 2.0;
    let until = |b: f64| (b - half - t).rem_euclid(DAY_MIN);
    Some(until(start).min(until(end)) * 60_000.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SunTimes {
    /// Local minutes after midnight.
    Times { sunrise: f64, sunset: f64 },
    PolarDay,
    PolarNight,
}

/// Sunrise and sunset (NOAA's general solar position equations, with the
/// standard -0.833° for refraction and the solar disc, evaluated at solar
/// noon, so a few minutes out) on day of year `doy`
/// at `lat`/`lon` (degrees, east-positive), in local minutes for a UTC
/// offset of `gmtoff_min`.
pub fn sun_times(lat: f64, lon: f64, doy: u32, gmtoff_min: f64) -> SunTimes {
    let g = 2.0 * std::f64::consts::PI / 365.0 * (doy as f64 - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let lat_r = lat.to_radians();
    let cos_ha = (90.833f64).to_radians().cos() / (lat_r.cos() * decl.cos()) - lat_r.tan() * decl.tan();
    if cos_ha > 1.0 {
        return SunTimes::PolarNight;
    }
    if cos_ha < -1.0 {
        return SunTimes::PolarDay;
    }
    let ha = cos_ha.acos().to_degrees();
    let local = |utc: f64| (utc + gmtoff_min).rem_euclid(DAY_MIN);
    SunTimes::Times {
        sunrise: local(720.0 - 4.0 * (lon + ha) - eqtime),
        sunset: local(720.0 - 4.0 * (lon - ha) - eqtime),
    }
}

/// The local clock: minutes after midnight, day of year (1-366), and the
/// UTC offset in minutes.
#[derive(Debug, Clone, Copy)]
struct LocalNow {
    minute: f64,
    doy: u32,
    gmtoff_min: f64,
}

fn local_now() -> LocalNow {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    // SAFETY: a zeroed `tm` is a valid out-parameter, and both pointers are
    // to locals.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    LocalNow {
        minute: tm.tm_hour as f64 * 60.0 + tm.tm_min as f64 + (tm.tm_sec as f64 + now.subsec_millis() as f64 / 1000.0) / 60.0,
        doy: tm.tm_yday as u32 + 1,
        gmtoff_min: tm.tm_gmtoff as f64 / 60.0,
    }
}

/// Where the schedule stands at `now`: the night factor and when it next
/// moves (`None`: it is moving), plus the night's bounds for the status line.
struct ScheduleNow {
    factor: f64,
    ms_until_change: Option<f64>,
    bounds: Option<(f64, f64)>,
}

fn schedule_now(cfg: &NightLightConfig, now: LocalNow) -> Option<ScheduleNow> {
    let (start, end) = match cfg.schedule {
        Schedule::None => return None,
        Schedule::Fixed { start, end } => (start, end),
        Schedule::Sun { lat, lon } => match sun_times(lat, lon, now.doy, now.gmtoff_min) {
            SunTimes::Times { sunrise, sunset } => (sunset, sunrise),
            SunTimes::PolarDay => return Some(ScheduleNow { factor: 0.0, ms_until_change: Some(f64::MAX), bounds: None }),
            SunTimes::PolarNight => return Some(ScheduleNow { factor: 1.0, ms_until_change: Some(f64::MAX), bounds: None }),
        },
    };
    Some(ScheduleNow {
        factor: night_factor(now.minute, start, end, cfg.transition_min),
        ms_until_change: ms_until_transition(now.minute, start, end, cfg.transition_min),
        bounds: Some((start, end)),
    })
}

// ---------------------------------------------------------------------------
// State

struct Fade {
    from: f64,
    to: f64,
    started: Instant,
}

/// What the last step decided, for the status line.
#[derive(Default, Clone, Copy)]
struct Report {
    factor: f64,
    bounds: Option<(f64, f64)>,
}

struct State {
    /// `None`: no `night_light` block, so outputs are left alone (any
    /// still holding this module's LUT are handed back the identity).
    config: Option<NightLightConfig>,
    mode: Mode,
    /// The temperature outputs are being given, and the one it heads for.
    shown_k: f64,
    target_k: f64,
    fade: Option<Fade>,
    /// Bumped whenever outputs must (re)commit: a new temperature, a
    /// resume. An output that committed this generation has nothing to do.
    gen: u32,
    /// LUTs for `transforms_k`, one per gamma size; null at neutral.
    transforms: Vec<(usize, *mut ffi::wlr_color_transform)>,
    transforms_k: u32,
    gamma_manager: *mut ffi::wlr_gamma_control_manager_v1,
    timer: crate::event_source::Timer,
    report: Report,
}

impl State {
    fn new() -> Self {
        State {
            config: None,
            mode: Mode::Auto,
            shown_k: NEUTRAL_K as f64,
            target_k: NEUTRAL_K as f64,
            fade: None,
            gen: 1,
            transforms: Vec::new(),
            transforms_k: NEUTRAL_K,
            gamma_manager: std::ptr::null_mut(),
            timer: crate::event_source::Timer::new(),
            report: Report::default(),
        }
    }

    /// Show `k`: a new rounded temperature drops the old LUTs and bumps the
    /// generation. Whether it did.
    fn show(&mut self, k: f64) -> bool {
        self.shown_k = k;
        let rounded = k.round() as u32;
        if rounded == self.transforms_k {
            return false;
        }
        self.drop_transforms();
        self.transforms_k = rounded;
        self.gen = self.gen.wrapping_add(1).max(1);
        true
    }

    fn drop_transforms(&mut self) {
        for (_, tr) in self.transforms.drain(..) {
            // SAFETY: made by `transform_for`, which holds this reference;
            // an output committed with it holds its own.
            unsafe { ffi::wlr_color_transform_unref(tr) };
        }
    }

    /// This temperature's LUT for an output with `size`-entry ramps; null
    /// when it is neutral (the identity).
    fn transform_for(&mut self, size: usize) -> *mut ffi::wlr_color_transform {
        if self.transforms_k == NEUTRAL_K {
            return std::ptr::null_mut();
        }
        if let Some(&(_, tr)) = self.transforms.iter().find(|(s, _)| *s == size) {
            return tr;
        }
        let white = white_point(self.transforms_k as f64);
        let [r, g, b] = white.map(|gain| ramp_channel(size, gain));
        // SAFETY: three `size`-entry ramps, which wlroots copies.
        let tr = unsafe { ffi::wlr_color_transform_init_lut_3x1d(size, r.as_ptr(), g.as_ptr(), b.as_ptr()) };
        if !tr.is_null() {
            self.transforms.push((size, tr));
        }
        tr
    }

    /// Where the temperature should be now, from the mode and the schedule.
    fn target(&mut self, now: LocalNow) -> (f64, Option<f64>) {
        let Some(cfg) = self.config.clone() else {
            self.report = Report::default();
            return (NEUTRAL_K as f64, None);
        };
        let (day, night) = (cfg.day_k as f64, cfg.night_k as f64);
        let sched = schedule_now(&cfg, now);
        self.report = Report {
            factor: sched.as_ref().map_or(0.0, |s| s.factor),
            bounds: sched.as_ref().and_then(|s| s.bounds),
        };
        match (self.mode, sched) {
            (Mode::On, _) | (Mode::Auto, None) => (night, None),
            (Mode::Off, _) => (day, None),
            (Mode::Auto, Some(s)) => (blend_k(day, night, s.factor), Some(s.ms_until_change.unwrap_or(0.0))),
        }
    }

    /// Move toward the target. Whether outputs need a frame, and when to
    /// step next (`None`: nothing to wait for).
    fn step(&mut self, now: LocalNow, at: Instant) -> (bool, Option<i32>) {
        let (target, until_change) = self.target(now);
        let restart = match &self.fade {
            Some(f) => (f.to - target).abs() > JUMP_K,
            None => (self.shown_k - target).abs() > JUMP_K,
        };
        if restart {
            self.fade = cce_core::motion::enabled().then(|| Fade { from: self.shown_k, to: target, started: at });
        }
        self.target_k = target;
        let shown = match &mut self.fade {
            Some(f) => {
                f.to = target;
                let p = (at.duration_since(f.started).as_secs_f64() * 1000.0 / FADE_MS).clamp(0.0, 1.0);
                let eased = p * p * (3.0 - 2.0 * p);
                let k = 1e6 / (to_mired(f.from) + (to_mired(f.to) - to_mired(f.from)) * eased);
                if p >= 1.0 {
                    self.fade = None;
                }
                k
            }
            None => target,
        };
        let changed = self.show(shown);
        let next = if self.fade.is_some() {
            Some(FADE_STEP_MS)
        } else {
            until_change.map(|ms| {
                if ms <= 0.0 { TRANSITION_STEP_MS } else { ms.clamp(1000.0, IDLE_RECHECK_MS as f64) as i32 }
            })
        };
        (changed, next)
    }
}

extern "C" {
    /// POSIX; the libc crate does not declare it. `localtime_r` need not
    /// re-read the zone, so a config load (and so login) does.
    fn tzset();
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

/// Record the gamma-control manager whose clients outrank this module
/// (`OutputManager::init`); null at deinit.
pub fn set_gamma_manager(manager: *mut ffi::wlr_gamma_control_manager_v1) {
    STATE.with(|s| s.borrow_mut().gamma_manager = manager);
}

/// Apply a config load: `None` when config.kdl has no `night_light` block.
/// The schedule's mode replaces any `ccectl` override.
pub fn configure(wm: &mut crate::window_manager::WindowManager, config: Option<NightLightConfig>) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.timer.is_set() {
            // SAFETY: `on_timer` is an event-loop entry point that reads no
            // data.
            s.timer = unsafe { crate::event_source::Timer::on_shared_loop(on_timer, 0) };
        }
        s.mode = config.as_ref().map_or(Mode::Auto, |c| c.mode);
        s.config = config;
        // Every output re-evaluates against the new block, even when the
        // temperature it shows stays where it was.
        s.gen = s.gen.wrapping_add(1).max(1);
    });
    // SAFETY: tzset only re-reads TZ and /etc/localtime.
    unsafe { tzset() };
    kick(wm);
}

/// The session came back (resume, VT switch): the clock may have jumped,
/// and the CRTCs may have lost their LUTs, so step soon and re-send.
pub fn session_active() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.gen = s.gen.wrapping_add(1).max(1);
        s.timer.arm(1);
    });
}

/// Step now, ask every output that has to change for a frame, and arm the
/// next step.
fn kick(wm: &mut crate::window_manager::WindowManager) {
    let (changed, next) = STATE.with(|s| s.borrow_mut().step(local_now(), Instant::now()));
    let _ = changed;
    for output in wm.outputs.values() {
        if wants_commit(&output.night_light, output.wlr_output()) {
            output.scene_output().damage_whole();
        }
    }
    STATE.with(|s| {
        let s = s.borrow();
        match next {
            Some(ms) => s.timer.arm(ms.max(1)),
            None => s.timer.disarm(),
        }
    });
}

unsafe extern "C" fn on_timer(_data: *mut std::ffi::c_void) -> std::ffi::c_int {
    let wm = &mut *crate::reentry::wm(crate::shared::server_ptr());
    kick(wm);
    0
}

// ---------------------------------------------------------------------------
// Per output

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputStatus {
    /// Never asked (no `night_light` block, or no frame yet).
    #[default]
    Idle,
    Applied,
    Neutral,
    /// No hardware gamma LUT.
    Unsupported,
    /// A gamma-control client holds the output.
    Client,
    /// The test commit refused the LUT.
    Refused,
}

impl OutputStatus {
    fn name(self) -> &'static str {
        match self {
            OutputStatus::Idle => "idle",
            OutputStatus::Applied => "applied",
            OutputStatus::Neutral => "neutral",
            OutputStatus::Unsupported => "unsupported",
            OutputStatus::Client => "client",
            OutputStatus::Refused => "refused",
        }
    }
}

/// An output's side: which generation it last committed, whether it holds
/// this module's LUT, and whether a client held it at the last frame.
/// Cells, so the modeset path (which has the store only by `&`) can
/// [`forget`](Self::forget).
#[derive(Default)]
pub struct PerOutput {
    sent_gen: Cell<u32>,
    owned: Cell<bool>,
    client: Cell<bool>,
    status: Cell<OutputStatus>,
}

impl PerOutput {
    /// The output was modeset: whatever its CRTC held is not to be trusted,
    /// so re-send at the next frame.
    pub fn forget(&self) {
        self.sent_gen.set(0);
    }
}

/// A colour transform [`prepare`] put into a frame, to be recorded with
/// [`committed`] once the commit takes.
pub struct Pending {
    gen: u32,
    owned: bool,
    status: OutputStatus,
}

fn client_holds(manager: *mut ffi::wlr_gamma_control_manager_v1, output: &crate::wlr::Wlr<ffi::wlr_output>) -> bool {
    // SAFETY: the manager is the live one `OutputManager::init` made (null
    // after deinit) and the output is the field's live one.
    !manager.is_null() && !output.is_null() && unsafe { !ffi::wlr_gamma_control_manager_v1_get_control(manager, output.ptr()).is_null() }
}

/// Whether this output has a LUT change to commit, so a frame with no
/// damage must still be drawn.
pub fn wants_commit(per: &PerOutput, output: &crate::wlr::Wlr<ffi::wlr_output>) -> bool {
    STATE.with(|s| {
        let s = s.borrow();
        if per.client.get() {
            return !client_holds(s.gamma_manager, output);
        }
        per.sent_gen.get() != s.gen && (s.config.is_some() || per.owned.get())
    })
}

/// Add this output's LUT to the frame in `state`, if it has one to send.
/// Call after the scene built the state (whose gamma-control commit it must
/// follow), and hand the result to [`committed`] if the commit takes.
pub fn prepare(per: &PerOutput, output: &crate::wlr::Wlr<ffi::wlr_output>, state: &mut crate::wlr::OutputStateBuf) -> Option<Pending> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let gen = s.gen;
        if per.sent_gen.get() == gen && !per.client.get() {
            return None;
        }
        if s.config.is_none() && !per.owned.get() {
            per.sent_gen.set(gen);
            per.status.set(OutputStatus::Idle);
            return None;
        }
        let size = output.gamma_size();
        if size == 0 {
            if per.status.get() != OutputStatus::Unsupported {
                log::info!("night light: {} has no gamma LUT; left alone", output.name());
            }
            per.sent_gen.set(gen);
            per.status.set(OutputStatus::Unsupported);
            return None;
        }
        if client_holds(s.gamma_manager, output) {
            if !per.client.get() {
                log::info!("night light: a gamma-control client holds {}; standing aside", output.name());
            }
            per.client.set(true);
            per.owned.set(false);
            per.sent_gen.set(gen);
            per.status.set(OutputStatus::Client);
            return None;
        }
        if per.client.replace(false) {
            log::info!("night light: the gamma-control client let go of {}", output.name());
        }
        let tr = if s.config.is_some() { s.transform_for(size) } else { std::ptr::null_mut() };
        if tr.is_null() && !per.owned.get() && output.color_transform().is_null() {
            per.sent_gen.set(gen);
            per.status.set(if s.config.is_some() { OutputStatus::Neutral } else { OutputStatus::Idle });
            return None;
        }
        // SAFETY: `tr` is null or one of `s.transforms`, live while held there.
        if !unsafe { state.try_color_transform(output, tr) } {
            if per.status.get() != OutputStatus::Refused {
                log::warn!("night light: {} refused a {}-entry gamma LUT", output.name(), size);
            }
            per.sent_gen.set(gen);
            per.status.set(OutputStatus::Refused);
            return None;
        }
        let status = if tr.is_null() { OutputStatus::Neutral } else { OutputStatus::Applied };
        Some(Pending { gen, owned: !tr.is_null(), status })
    })
}

/// The frame carrying [`prepare`]'s LUT was committed.
pub fn committed(per: &PerOutput, pending: Pending) {
    per.sent_gen.set(pending.gen);
    per.owned.set(pending.owned);
    per.status.set(pending.status);
}

// ---------------------------------------------------------------------------
// ccectl night-light

pub fn ipc(wm: &mut crate::window_manager::WindowManager, args: &[&str]) -> String {
    let err = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let configured = s.config.is_some();
        match args {
            [] | ["status"] => None,
            [_, ..] if !configured => Some("error: no night_light block in config.kdl\n".to_string()),
            ["toggle"] => {
                // Toggle against where it is heading: nearer night goes to day.
                let (night, day) = s.config.as_ref().map_or((0.0, 0.0), |c| (c.night_k as f64, c.day_k as f64));
                let target = to_mired(s.target_k);
                let nearer_night = (target - to_mired(night)).abs() < (target - to_mired(day)).abs();
                s.mode = if nearer_night { Mode::Off } else { Mode::On };
                None
            }
            [m] if Mode::parse(m).is_some() => {
                s.mode = Mode::parse(m).unwrap_or(Mode::Auto);
                None
            }
            ["temperature", k] => match k.parse::<i64>() {
                Ok(k) => {
                    if let Some(c) = s.config.as_mut() {
                        c.night_k = clamp_k(k);
                    }
                    None
                }
                Err(_) => Some("error: temperature must be an integer (kelvin)\n".to_string()),
            },
            _ => Some("error: usage: night-light [status|on|off|auto|toggle|temperature <K>]\n".to_string()),
        }
    });
    if let Some(e) = err {
        return e;
    }
    if !args.is_empty() && args != ["status"] {
        kick(wm);
    }
    status_line(wm)
}

fn status_line(wm: &crate::window_manager::WindowManager) -> String {
    let mut line = STATE.with(|s| {
        let s = s.borrow();
        let Some(cfg) = &s.config else {
            return "configured=no".to_string();
        };
        let schedule = match cfg.schedule {
            Schedule::None => "none".to_string(),
            Schedule::Fixed { .. } => "fixed".to_string(),
            Schedule::Sun { lat, lon } => format!("sun({:.3},{:.3})", lat, lon),
        };
        let bounds = match s.report.bounds {
            Some((start, end)) => format!(" night_from={} night_to={}", fmt_clock(start), fmt_clock(end)),
            None if matches!(cfg.schedule, Schedule::Sun { .. }) => " night_from=- night_to=- (polar)".to_string(),
            None => String::new(),
        };
        format!(
            "configured=yes mode={} temperature={} target={} night_temperature={} day_temperature={} schedule={}{} night={:.2} transition={}",
            s.mode.name(),
            s.shown_k.round(),
            s.target_k.round(),
            cfg.night_k,
            cfg.day_k,
            schedule,
            bounds,
            s.report.factor,
            cfg.transition_min,
        )
    });
    let outputs: Vec<String> = wm
        .outputs
        .values()
        .map(|o| format!("{}:{}", o.wlr_output().name(), o.night_light.status.get().name()))
        .collect();
    line.push_str(&format!(" outputs={}\n", if outputs.is_empty() { "-".to_string() } else { outputs.join(",") }));
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn neutral_is_identity() {
        assert_eq!(white_point(6500.0), [1.0, 1.0, 1.0]);
        let ramp = ramp_channel(256, 1.0);
        for (i, v) in ramp.iter().enumerate() {
            assert!(close(*v as f64, i as f64 * 65535.0 / 255.0, 1.0), "entry {i}: {v}");
        }
    }

    /// gammastep's (redshift's) table, the gamma-encoded white point, at a
    /// few temperatures: what "N kelvin" looks like elsewhere.
    #[test]
    fn white_point_matches_gammastep() {
        let reference: &[(f64, [f64; 3])] = &[
            (2000.0, [1.0, 0.5436, 0.0868]),
            (3000.0, [1.0, 0.7198, 0.4286]),
            (3500.0, [1.0, 0.7799, 0.5464]),
            (4000.0, [1.0, 0.8285, 0.6482]),
            (5000.0, [1.0, 0.9020, 0.8147]),
            (6000.0, [1.0, 0.9711, 0.9431]),
            (7000.0, [0.9516, 0.9698, 1.0]),
            (10000.0, [0.7899, 0.8649, 1.0]),
        ];
        for &(k, want) in reference {
            let got = white_point(k).map(srgb_encode);
            for c in 0..3 {
                assert!(close(got[c], want[c], 0.03), "{k} K channel {c}: {} vs {}", got[c], want[c]);
            }
        }
    }

    #[test]
    fn warmer_is_monotonic() {
        let mut prev = white_point(MIN_K as f64);
        for k in (MIN_K..=NEUTRAL_K).step_by(100).skip(1) {
            let wp = white_point(k as f64);
            assert_eq!(wp[0], 1.0, "{k} K: red stays at 1");
            assert!(wp[1] >= prev[1] && wp[2] >= prev[2], "{k} K: green and blue rise toward neutral");
            prev = wp;
        }
    }

    #[test]
    fn ramp_scales_linear_light() {
        let ramp = ramp_channel(1024, 0.25);
        assert_eq!(ramp[0], 0);
        assert!(ramp.windows(2).all(|w| w[0] <= w[1]));
        // Full scale lands at encode(0.25) ≈ 0.5371.
        assert!(close(ramp[1023] as f64 / 65535.0, srgb_encode(0.25), 1e-4));
    }

    #[test]
    fn blend_is_in_mireds() {
        assert!(close(blend_k(6500.0, 3500.0, 0.0), 6500.0, 1e-9));
        assert!(close(blend_k(6500.0, 3500.0, 1.0), 3500.0, 1e-9));
        // Halfway in mireds: 1e6 / ((153.85 + 285.71) / 2) ≈ 4550 K, not 5000.
        assert!(close(blend_k(6500.0, 3500.0, 0.5), 4550.0, 1.0));
    }

    #[test]
    fn fixed_night_fades_across_midnight() {
        let (start, end, d) = (21.0 * 60.0, 7.0 * 60.0, 60.0);
        let f = |h: f64| night_factor(h * 60.0, start, end, d);
        assert_eq!(f(12.0), 0.0);
        assert_eq!(f(20.4), 0.0);
        assert!(close(f(21.0), 0.5, 1e-9));
        assert!(close(f(20.75), 0.25, 1e-9));
        assert_eq!(f(21.5), 1.0);
        assert_eq!(f(0.0), 1.0);
        assert_eq!(f(3.0), 1.0);
        assert!(close(f(7.0), 0.5, 1e-9));
        assert!(close(f(7.25), 0.25, 1e-9));
        assert_eq!(f(7.5), 0.0);
        // No night at all when start == end.
        assert_eq!(night_factor(600.0, 300.0, 300.0, 30.0), 0.0);
        // A transition longer than the night is cut to fit it.
        assert!(night_factor(22.0 * 60.0, 21.0 * 60.0, 23.0 * 60.0, 600.0) > 0.99);
    }

    #[test]
    fn next_transition() {
        let (start, end, d) = (21.0 * 60.0, 7.0 * 60.0, 60.0);
        // At noon the next fade starts at 20:30.
        assert!(close(ms_until_transition(720.0, start, end, d).unwrap(), 510.0 * 60_000.0, 1.0));
        // At midnight, 06:30.
        assert!(close(ms_until_transition(0.0, start, end, d).unwrap(), 390.0 * 60_000.0, 1.0));
        assert_eq!(ms_until_transition(21.0 * 60.0, start, end, d), None);
    }

    #[test]
    fn sun_times_new_york_midsummer() {
        // 2026-06-21, New York (EDT, UTC-4): sunrise 05:25, sunset 20:31.
        let SunTimes::Times { sunrise, sunset } = sun_times(40.7128, -74.0060, 172, -240.0) else {
            panic!("no sunrise");
        };
        assert!(close(sunrise, 5.0 * 60.0 + 25.0, 3.0), "sunrise {}", fmt_clock(sunrise));
        assert!(close(sunset, 20.0 * 60.0 + 31.0, 3.0), "sunset {}", fmt_clock(sunset));
    }

    #[test]
    fn sun_times_polar() {
        assert_eq!(sun_times(78.2, 15.6, 172, 120.0), SunTimes::PolarDay);
        assert_eq!(sun_times(78.2, 15.6, 355, 60.0), SunTimes::PolarNight);
    }

    #[test]
    fn clock_parsing() {
        assert_eq!(parse_clock("21:30"), Some(1290.0));
        assert_eq!(parse_clock("07:00"), Some(420.0));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("7"), None);
        assert_eq!(fmt_clock(1290.0), "21:30");
        assert_eq!(fmt_clock(-30.0), "23:30");
    }

    fn now_at(minute: f64) -> LocalNow {
        LocalNow { minute, doy: 172, gmtoff_min: 0.0 }
    }

    #[test]
    fn step_follows_the_schedule_and_fades_jumps() {
        let mut s = State::new();
        s.config = Some(NightLightConfig {
            night_k: 3500,
            schedule: Schedule::Fixed { start: 21.0 * 60.0, end: 7.0 * 60.0 },
            transition_min: 60.0,
            ..Default::default()
        });
        let t0 = Instant::now();
        // Midday: neutral, nothing to change, sleep until the evening.
        let (changed, next) = s.step(now_at(720.0), t0);
        assert!(!changed);
        assert_eq!(next, Some(IDLE_RECHECK_MS));
        assert_eq!(s.transforms_k, NEUTRAL_K);
        // Into the evening transition: the first step is a jump from
        // neutral, which fades; once it has, the schedule is followed as it
        // moves, with no further fade.
        s.step(now_at(21.0 * 60.0), t0);
        let settled = t0 + std::time::Duration::from_millis(FADE_MS as u64 + 10);
        let (_, next) = s.step(now_at(21.0 * 60.0), settled);
        assert!(close(s.shown_k, blend_k(6500.0, 3500.0, 0.5), 1.0));
        assert_eq!(next, Some(TRANSITION_STEP_MS));
        s.step(now_at(21.0 * 60.0 + 1.0), settled);
        assert!(s.fade.is_none(), "a minute's step is not a jump");
        assert!(close(s.shown_k, blend_k(6500.0, 3500.0, 0.5 + 1.0 / 60.0), 1.0));
        // Forced on at midday: a jump, which fades over FADE_MS.
        let mut s2 = State::new();
        s2.config = s.config.clone();
        s2.mode = Mode::On;
        let gen0 = s2.gen;
        let (_, next) = s2.step(now_at(720.0), t0);
        let fading = s2.fade.is_some();
        if fading {
            assert_eq!(next, Some(FADE_STEP_MS));
            let later = t0 + std::time::Duration::from_millis(FADE_MS as u64 + 10);
            let (_, next) = s2.step(now_at(720.0), later);
            assert_eq!(next, None, "a fixed mode waits for nothing");
        }
        assert_eq!(s2.shown_k.round(), 3500.0);
        assert_ne!(s2.gen, gen0);
    }
}
