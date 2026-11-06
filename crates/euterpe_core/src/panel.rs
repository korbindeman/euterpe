//! Hardware controls: pots, switches and buttons bound to parameters.
//!
//! A device declares its panel as data ([`Device::PANEL`](crate::Device::PANEL)):
//!
//! ```
//! use euterpe_core::panel::{Binding, button, pot, switch};
//!
//! const PANEL: &[Binding] = &[
//!     pot(0, "drive"),       // a pot sweeps a float through its taper
//!     pot(1, "level"),
//!     switch(0, "bright"),   // an on/off switch holds a toggle
//!     button(1, "voice"),    // a button flips a toggle or steps a choice
//! ];
//! ```
//!
//! A platform reads its controls about once a millisecond into [`Readings`]
//! and hands them to a [`Panel`], which smooths the pots, debounces the
//! switches and sets the parameters. The panel is checked against the
//! parameter schema at compile time ([`check`]).
//!
//! - **Pots** are smoothed (about 5 ms at 1 kHz) and have hysteresis, so ADC
//!   noise never moves a parameter; a pot that is turned moves it at once.
//!   The ends snap, so the whole range is reachable.
//! - **Switches** (maintained) set a toggle, or a two-way choice, to their
//!   position, which is read once at start-up too.
//! - **Buttons** (momentary) flip a toggle or step a choice on each press.
//!
//! Switch and button inputs are debounced: a change counts after
//! [`DEBOUNCE_TICKS`] equal readings in a row.

use crate::params::{ParamDesc, ParamKind, ParamSet, ParamValue};

/// Pots a panel can bind: `0..MAX_POTS`.
pub const MAX_POTS: usize = 8;
/// Switch or button inputs a panel can bind: `0..MAX_SWITCHES`.
pub const MAX_SWITCHES: usize = 8;
/// Readings in a row before a switch or button counts as changed: 5 ms at
/// a 1 kHz scan, longer than a typical contact bounce.
pub const DEBOUNCE_TICKS: u8 = 5;
/// How far (of the full 0..1 travel) a pot must move before its parameter
/// does: about 1/250, above the noise of a 12-bit-effective ADC.
pub const POT_HYSTERESIS: f32 = 0.004;
/// One-pole coefficient for pot readings per scan: about 5 ms at 1 kHz.
const POT_SMOOTHING: f32 = 0.2;

/// A physical control, by its index on the platform's panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// A potentiometer (or any 0..1 analog input).
    Pot(u8),
    /// A maintained on/off switch.
    Switch(u8),
    /// A momentary button or footswitch.
    Button(u8),
}

/// One control bound to one parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub control: Control,
    /// The parameter's full id, e.g. `"drive.level"`.
    pub param: &'static str,
}

/// Pot `index` sweeps `param` through its range and taper.
#[must_use]
pub const fn pot(index: u8, param: &'static str) -> Binding {
    Binding {
        control: Control::Pot(index),
        param,
    }
}

/// Switch input `index`, maintained: closed sets a toggle on, or a choice
/// to its second option.
#[must_use]
pub const fn switch(index: u8, param: &'static str) -> Binding {
    Binding {
        control: Control::Switch(index),
        param,
    }
}

/// Switch input `index`, momentary: each press flips a toggle or steps a
/// choice to its next option (wrapping).
#[must_use]
pub const fn button(index: u8, param: &'static str) -> Binding {
    Binding {
        control: Control::Button(index),
        param,
    }
}

/// What is wrong with a panel, and which binding (by position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelError {
    /// The binding names no parameter in the schema.
    UnknownParam(usize),
    /// The control index is past [`MAX_POTS`] or [`MAX_SWITCHES`].
    NoSuchControl(usize),
    /// A switch or button bound to a float, or a switch to a choice without
    /// two options.
    WrongKind(usize),
    /// One switch input bound both as a switch and as a button.
    MixedInput(usize),
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn find(schema: &[ParamDesc], id: &str) -> Option<ParamKind> {
    let mut i = 0;
    while i < schema.len() {
        if str_eq(schema[i].id, id) {
            return Some(schema[i].kind);
        }
        i += 1;
    }
    None
}

/// Check a panel against a parameter schema. `const`, so a platform can
/// fail the build on a bad panel:
///
/// ```
/// # use euterpe_core::panel::{check, pot};
/// # use euterpe_core::ParamSet;
/// const _: () = assert!(check(&[pot(0, "x")], <() as ParamSet>::SCHEMA).is_err());
/// ```
///
/// # Errors
///
/// The first binding that is wrong, and how.
pub const fn check(panel: &[Binding], schema: &[ParamDesc]) -> Result<(), PanelError> {
    let mut i = 0;
    while i < panel.len() {
        let b = panel[i];
        let Some(kind) = find(schema, b.param) else {
            return Err(PanelError::UnknownParam(i));
        };
        match b.control {
            Control::Pot(p) => {
                if p as usize >= MAX_POTS {
                    return Err(PanelError::NoSuchControl(i));
                }
            }
            Control::Switch(s) | Control::Button(s) => {
                if s as usize >= MAX_SWITCHES {
                    return Err(PanelError::NoSuchControl(i));
                }
                let is_switch = matches!(b.control, Control::Switch(_));
                let ok = match kind {
                    ParamKind::Float { .. } => false,
                    ParamKind::Toggle { .. } => true,
                    ParamKind::Choice { options, .. } => !is_switch || options.len() >= 2,
                };
                if !ok {
                    return Err(PanelError::WrongKind(i));
                }
                // The same input can't be both maintained and momentary.
                let mut j = 0;
                while j < panel.len() {
                    match (b.control, panel[j].control) {
                        (Control::Switch(a), Control::Button(c))
                        | (Control::Button(a), Control::Switch(c))
                            if a == c =>
                        {
                            return Err(PanelError::MixedInput(i));
                        }
                        _ => {}
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    Ok(())
}

/// One scan of the controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Readings {
    /// Pot positions, `0.0..=1.0` (the platform scales its ADC counts).
    pub pots: [f32; MAX_POTS],
    /// Switch inputs, bit `i` set while input `i` is closed (pressed).
    pub switches: u8,
}

impl Default for Readings {
    fn default() -> Self {
        Self {
            pots: [0.0; MAX_POTS],
            switches: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PotState {
    smoothed: f32,
    /// The position last handed to the parameters.
    value: f32,
}

impl PotState {
    /// Smooth the reading; `true` if the pot moved past the hysteresis.
    fn scan(&mut self, reading: f32) -> bool {
        let x = reading.clamp(0.0, 1.0);
        self.smoothed += POT_SMOOTHING * (x - self.smoothed);
        // The ends snap, so the hysteresis never keeps a pot off 0 or 1.
        let (target, at_end) = if self.smoothed < 0.5 * POT_HYSTERESIS {
            (0.0, true)
        } else if self.smoothed > 1.0 - 0.5 * POT_HYSTERESIS {
            (1.0, true)
        } else {
            (self.smoothed, false)
        };
        let delta = (target - self.value).abs();
        let moved = delta > POT_HYSTERESIS || (at_end && delta > 0.0);
        if moved {
            self.value = target;
        }
        moved
    }
}

/// An integrating debouncer: the count runs toward the raw input, and the
/// state changes only at the ends.
#[derive(Clone, Copy, Debug, Default)]
struct Debounce {
    count: u8,
    closed: bool,
}

impl Debounce {
    /// `Some(new state)` when the debounced state changes.
    fn scan(&mut self, raw: bool) -> Option<bool> {
        if raw {
            self.count = (self.count + 1).min(DEBOUNCE_TICKS);
        } else {
            self.count = self.count.saturating_sub(1);
        }
        let closed = if self.count == DEBOUNCE_TICKS {
            true
        } else if self.count == 0 {
            false
        } else {
            self.closed
        };
        (closed != self.closed).then(|| {
            self.closed = closed;
            closed
        })
    }
}

/// The control-side state of a panel. Plain data, so it lives anywhere; a
/// platform keeps one next to its own copy of the parameters.
#[derive(Clone, Debug, Default)]
pub struct Panel {
    pots: [PotState; MAX_POTS],
    switches: [Debounce; MAX_SWITCHES],
    started: bool,
}

impl Panel {
    /// Take one scan and update `params`; `true` if any parameter changed.
    ///
    /// The first scan applies every pot and switch as it is, so the device
    /// starts where the panel is set.
    pub fn update<P: ParamSet>(&mut self, panel: &[Binding], r: &Readings, params: &mut P) -> bool {
        let mut moved = [false; MAX_POTS];
        let mut flipped = [None; MAX_SWITCHES];
        let first = !self.started;
        if self.started {
            for ((pot, &x), m) in self.pots.iter_mut().zip(&r.pots).zip(&mut moved) {
                *m = pot.scan(x);
            }
            for (i, (sw, f)) in self.switches.iter_mut().zip(&mut flipped).enumerate() {
                *f = sw.scan(r.switches & (1 << i) != 0);
            }
        } else {
            self.started = true;
            for ((pot, &x), m) in self.pots.iter_mut().zip(&r.pots).zip(&mut moved) {
                let x = x.clamp(0.0, 1.0);
                *pot = PotState {
                    smoothed: x,
                    value: x,
                };
                *m = true;
            }
            for (i, sw) in self.switches.iter_mut().enumerate() {
                let closed = r.switches & (1 << i) != 0;
                *sw = Debounce {
                    count: if closed { DEBOUNCE_TICKS } else { 0 },
                    closed,
                };
            }
        }

        let mut changed = false;
        for b in panel {
            changed |= match b.control {
                Control::Pot(i) => match (moved.get(usize::from(i)), self.pots.get(usize::from(i)))
                {
                    (Some(true), Some(pot)) => params.set_normalized(b.param, pot.value).is_ok(),
                    _ => false,
                },
                Control::Switch(i) => {
                    let state = match flipped.get(usize::from(i)) {
                        Some(&Some(closed)) => Some(closed),
                        // At start-up a switch applies its position.
                        _ if first => self.switches.get(usize::from(i)).map(|s| s.closed),
                        _ => None,
                    };
                    state.is_some_and(|closed| set_switch(params, b.param, closed))
                }
                Control::Button(i) => {
                    matches!(flipped.get(usize::from(i)), Some(&Some(true)))
                        && press(params, b.param)
                }
            };
        }
        changed
    }
}

fn set_switch<P: ParamSet>(params: &mut P, id: &str, closed: bool) -> bool {
    let value = match P::desc(id).map(|d| d.kind) {
        Some(ParamKind::Toggle { .. }) => ParamValue::Bool(closed),
        Some(ParamKind::Choice { .. }) => ParamValue::Choice(u8::from(closed)),
        _ => return false,
    };
    params.set(id, value).is_ok()
}

fn press<P: ParamSet>(params: &mut P, id: &str) -> bool {
    let next = match (P::desc(id).map(|d| d.kind), params.get(id)) {
        (_, Some(ParamValue::Bool(on))) => ParamValue::Bool(!on),
        (Some(ParamKind::Choice { options, .. }), Some(ParamValue::Choice(i))) => {
            let next = usize::from(i) + 1;
            ParamValue::Choice(if next < options.len() {
                u8::try_from(next).unwrap_or(0)
            } else {
                0
            })
        }
        _ => return false,
    };
    params.set(id, next).is_ok()
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "pots land on exact values")]
mod tests {
    use super::*;
    use crate::Params;

    const SHAPES: &[&str] = &["Soft", "Hard", "Fold"];

    #[derive(Params, Clone, Debug, PartialEq)]
    struct TestParams {
        #[param(id = "gain", label = "Gain", kind = ParamKind::float(0.0, 10.0, 5.0))]
        gain: f32,
        #[param(id = "on", label = "On", kind = ParamKind::toggle(false))]
        on: bool,
        #[param(id = "shape", label = "Shape", kind = ParamKind::choice(SHAPES, 0))]
        shape: u8,
    }

    const PANEL: &[Binding] = &[pot(2, "gain"), switch(0, "on"), button(3, "shape")];
    const _: () = assert!(check(PANEL, TestParams::SCHEMA).is_ok());

    fn readings(pot2: f32, switches: u8) -> Readings {
        let mut r = Readings::default();
        r.pots[2] = pot2;
        r.switches = switches;
        r
    }

    #[test]
    fn check_finds_each_mistake() {
        let s = TestParams::SCHEMA;
        assert_eq!(
            check(&[pot(0, "gian")], s),
            Err(PanelError::UnknownParam(0))
        );
        assert_eq!(
            check(&[pot(8, "gain")], s),
            Err(PanelError::NoSuchControl(0))
        );
        assert_eq!(
            check(&[button(8, "on")], s),
            Err(PanelError::NoSuchControl(0))
        );
        assert_eq!(
            check(&[pot(0, "on"), switch(1, "gain")], s),
            Err(PanelError::WrongKind(1))
        );
        assert_eq!(
            check(&[switch(1, "on"), button(1, "shape")], s),
            Err(PanelError::MixedInput(0))
        );
        // A pot may drive a toggle or a choice, and a switch a choice.
        assert!(check(&[pot(0, "on"), pot(1, "shape"), switch(2, "shape")], s).is_ok());
    }

    #[test]
    fn first_scan_applies_the_panel_as_set() {
        let mut p = TestParams::default();
        let mut panel = Panel::default();
        assert!(panel.update(PANEL, &readings(0.3, 0b1), &mut p));
        assert_eq!(p.gain, 3.0);
        assert!(p.on);
        assert_eq!(p.shape, 0, "a held button is not a press");
    }

    #[test]
    fn adc_noise_never_moves_a_parameter() {
        let mut p = TestParams::default();
        let mut panel = Panel::default();
        panel.update(PANEL, &readings(0.5, 0), &mut p);
        let mut s = 1_u32;
        for _ in 0..10_000 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            // ±0.3% of noise: about ±12 counts of a 12-bit ADC.
            #[expect(clippy::cast_precision_loss, reason = "test noise")]
            let noise = (s as f32 / u32::MAX as f32 - 0.5) * 0.006;
            assert!(!panel.update(PANEL, &readings(0.5 + noise, 0), &mut p));
        }
        assert_eq!(p.gain, 5.0);
    }

    #[test]
    fn a_turned_pot_follows_and_reaches_both_ends() {
        let mut p = TestParams::default();
        let mut panel = Panel::default();
        panel.update(PANEL, &readings(0.5, 0), &mut p);
        for _ in 0..50 {
            panel.update(PANEL, &readings(0.8, 0), &mut p);
        }
        assert!((p.gain - 8.0).abs() < 0.05, "{}", p.gain);
        for _ in 0..100 {
            panel.update(PANEL, &readings(1.0, 0), &mut p);
        }
        assert_eq!(p.gain, 10.0);
        for _ in 0..100 {
            panel.update(PANEL, &readings(0.0, 0), &mut p);
        }
        assert_eq!(p.gain, 0.0);
    }

    #[test]
    fn a_bouncing_switch_changes_once() {
        let mut p = TestParams::default();
        let mut panel = Panel::default();
        panel.update(PANEL, &readings(0.5, 0), &mut p);
        // Close with 3 ms of bounce, then hold.
        let bounce = [1, 0, 1, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1];
        let mut changes = 0;
        for &b in &bounce {
            if panel.update(PANEL, &readings(0.5, b), &mut p) {
                changes += 1;
            }
        }
        assert_eq!(changes, 1);
        assert!(p.on);
    }

    #[test]
    fn a_button_steps_a_choice_once_per_press_and_wraps() {
        let mut p = TestParams::default();
        let mut panel = Panel::default();
        panel.update(PANEL, &readings(0.5, 0), &mut p);
        let press = |panel: &mut Panel, p: &mut TestParams| {
            for _ in 0..20 {
                panel.update(PANEL, &readings(0.5, 0b1000), p);
            }
            for _ in 0..20 {
                panel.update(PANEL, &readings(0.5, 0), p);
            }
        };
        for want in [1, 2, 0, 1] {
            press(&mut panel, &mut p);
            assert_eq!(p.shape, want);
        }
        assert!(!p.on, "other inputs untouched");
    }
}
