//! Parameters: data with a static, introspectable schema.
//!
//! A [`ParamSet`] is a plain struct of current values plus a `SCHEMA` that
//! describes each one (id, label, range, unit, taper, smoothing time). The
//! same schema drives UIs, hardware bindings, presets, the CLI and agents.
//!
//! Declare one with [`crate::params!`]:
//!
//! ```
//! use euterpe_core::{params, ParamKind, ParamSet, ParamValue};
//!
//! params! {
//!     pub struct DriveParams {
//!         drive: f32 = ("drive", "Drive", ParamKind::float(0.0, 24.0, 12.0).unit("dB").smooth_ms(20.0)),
//!         enabled: bool = ("enabled", "Enabled", ParamKind::toggle(true)),
//!     }
//! }
//!
//! let mut p = DriveParams::default();
//! assert_eq!(p.drive, 12.0);
//! p.set_str("drive", "30").unwrap(); // clamped to the range
//! assert_eq!(p.get("drive"), Some(ParamValue::Float(24.0)));
//! assert_eq!(DriveParams::SCHEMA.len(), 2);
//! ```

use crate::fastmath;

/// How a continuous parameter maps to a normalized 0..1 control (a pot, a slider).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taper {
    Linear,
    /// Equal ratios per equal travel. Needs `min > 0` (frequencies, times).
    Exponential,
}

/// The type and range of one parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        default: f32,
        unit: &'static str,
        taper: Taper,
        /// Smoothing time processors should use for this parameter. 0 = none.
        smooth_ms: f32,
    },
    Toggle {
        default: bool,
    },
    Choice {
        options: &'static [&'static str],
        default: u8,
    },
}

impl ParamKind {
    pub const fn float(min: f32, max: f32, default: f32) -> Self {
        ParamKind::Float {
            min,
            max,
            default,
            unit: "",
            taper: Taper::Linear,
            smooth_ms: 0.0,
        }
    }

    pub const fn toggle(default: bool) -> Self {
        ParamKind::Toggle { default }
    }

    pub const fn choice(options: &'static [&'static str], default: u8) -> Self {
        ParamKind::Choice { options, default }
    }

    /// Set the unit label of a float parameter. No effect on other kinds.
    #[must_use]
    pub const fn unit(self, unit: &'static str) -> Self {
        match self {
            ParamKind::Float {
                min,
                max,
                default,
                taper,
                smooth_ms,
                ..
            } => ParamKind::Float {
                min,
                max,
                default,
                unit,
                taper,
                smooth_ms,
            },
            other => other,
        }
    }

    /// Set the taper of a float parameter. No effect on other kinds.
    #[must_use]
    pub const fn taper(self, taper: Taper) -> Self {
        match self {
            ParamKind::Float {
                min,
                max,
                default,
                unit,
                smooth_ms,
                ..
            } => ParamKind::Float {
                min,
                max,
                default,
                unit,
                taper,
                smooth_ms,
            },
            other => other,
        }
    }

    /// Set the smoothing time of a float parameter. No effect on other kinds.
    #[must_use]
    pub const fn smooth_ms(self, smooth_ms: f32) -> Self {
        match self {
            ParamKind::Float {
                min,
                max,
                default,
                unit,
                taper,
                ..
            } => ParamKind::Float {
                min,
                max,
                default,
                unit,
                taper,
                smooth_ms,
            },
            other => other,
        }
    }

    /// The default value as a [`ParamValue`].
    pub const fn default_value(&self) -> ParamValue {
        match *self {
            ParamKind::Float { default, .. } => ParamValue::Float(default),
            ParamKind::Toggle { default } => ParamValue::Bool(default),
            ParamKind::Choice { default, .. } => ParamValue::Choice(default),
        }
    }

    /// Map a value to 0..1 according to the range and taper.
    pub fn to_normalized(&self, value: ParamValue) -> f32 {
        match (*self, value) {
            (
                ParamKind::Float {
                    min, max, taper, ..
                },
                ParamValue::Float(v),
            ) => {
                let v = v.clamp(min, max);
                match taper {
                    Taper::Linear => (v - min) / (max - min),
                    Taper::Exponential => fastmath::log2(v / min) / fastmath::log2(max / min),
                }
            }
            (ParamKind::Toggle { .. }, ParamValue::Bool(b)) => f32::from(u8::from(b)),
            (ParamKind::Choice { options, .. }, ParamValue::Choice(i)) if options.len() > 1 => {
                f32::from(i) / (options.len() - 1) as f32
            }
            _ => 0.0,
        }
    }

    /// Map 0..1 back to a value according to the range and taper.
    pub fn from_normalized(&self, n: f32) -> ParamValue {
        let n = n.clamp(0.0, 1.0);
        match *self {
            ParamKind::Float {
                min, max, taper, ..
            } => ParamValue::Float(match taper {
                Taper::Linear => min + n * (max - min),
                Taper::Exponential => min * fastmath::exp2(n * fastmath::log2(max / min)),
            }),
            ParamKind::Toggle { .. } => ParamValue::Bool(n >= 0.5),
            ParamKind::Choice { options, .. } => {
                let last = options.len().saturating_sub(1) as f32;
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "n is clamped to [0, 1], so the result is in 0..=last; `as` saturates anyway"
                )]
                let index = (n * last + 0.5) as u8;
                ParamValue::Choice(index)
            }
        }
    }

    /// Parse a value from text: a number for floats; `true/false/on/off/1/0`
    /// for toggles; an option name (case-insensitive) or index for choices.
    pub fn parse(&self, s: &str) -> Result<ParamValue, ParamError> {
        let s = s.trim();
        match *self {
            ParamKind::Float { .. } => s
                .parse::<f32>()
                .map(ParamValue::Float)
                .map_err(|_| ParamError::Parse),
            ParamKind::Toggle { .. } => match s {
                _ if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("on") || s == "1" => {
                    Ok(ParamValue::Bool(true))
                }
                _ if s.eq_ignore_ascii_case("false")
                    || s.eq_ignore_ascii_case("off")
                    || s == "0" =>
                {
                    Ok(ParamValue::Bool(false))
                }
                _ => Err(ParamError::Parse),
            },
            ParamKind::Choice { options, .. } => {
                if let Some(i) = options.iter().position(|o| o.eq_ignore_ascii_case(s)) {
                    return u8::try_from(i)
                        .map(ParamValue::Choice)
                        .map_err(|_| ParamError::Parse);
                }
                match s.parse::<u8>() {
                    Ok(i) if (i as usize) < options.len() => Ok(ParamValue::Choice(i)),
                    _ => Err(ParamError::Parse),
                }
            }
        }
    }
}

/// A parameter value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Bool(bool),
    Choice(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamError {
    UnknownId,
    TypeMismatch,
    /// A float that is NaN or infinite, or a choice index past the options.
    Invalid,
    Parse,
}

impl core::fmt::Display for ParamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            ParamError::UnknownId => "unknown parameter id",
            ParamError::TypeMismatch => "value has the wrong type for this parameter",
            ParamError::Invalid => "value is not valid for this parameter",
            ParamError::Parse => "could not parse value",
        })
    }
}

/// Describes one parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamDesc {
    /// Stable machine id, e.g. `"drive"`. Composed devices prefix these (`"amp.drive"`).
    pub id: &'static str,
    /// Human-readable label.
    pub label: &'static str,
    pub kind: ParamKind,
}

/// A set of parameter values with a static schema.
///
/// Implement it with [`crate::params!`] rather than by hand.
pub trait ParamSet: Default + Clone {
    const SCHEMA: &'static [ParamDesc];

    fn get(&self, id: &str) -> Option<ParamValue>;

    /// Set a value. Floats are clamped to their range.
    fn set(&mut self, id: &str, value: ParamValue) -> Result<(), ParamError>;

    fn desc(id: &str) -> Option<&'static ParamDesc> {
        Self::SCHEMA.iter().find(|d| d.id == id)
    }

    /// The smoothing time declared for a float parameter, in ms (0 if none).
    fn smooth_ms(id: &str) -> f32 {
        match Self::desc(id).map(|d| d.kind) {
            Some(ParamKind::Float { smooth_ms, .. }) => smooth_ms,
            _ => 0.0,
        }
    }

    /// Parse `text` according to the parameter's kind and set it.
    fn set_str(&mut self, id: &str, text: &str) -> Result<(), ParamError> {
        let desc = Self::desc(id).ok_or(ParamError::UnknownId)?;
        let value = desc.kind.parse(text)?;
        self.set(id, value)
    }

    /// Set from a normalized 0..1 control position (a pot reading).
    fn set_normalized(&mut self, id: &str, n: f32) -> Result<(), ParamError> {
        let desc = Self::desc(id).ok_or(ParamError::UnknownId)?;
        self.set(id, desc.kind.from_normalized(n))
    }
}

impl ParamSet for () {
    const SCHEMA: &'static [ParamDesc] = &[];

    fn get(&self, _: &str) -> Option<ParamValue> {
        None
    }

    fn set(&mut self, _: &str, _: ParamValue) -> Result<(), ParamError> {
        Err(ParamError::UnknownId)
    }
}

/// A Rust type that can hold a parameter value. Used by [`crate::params!`].
pub trait ParamField: Sized {
    fn default_for(kind: &ParamKind) -> Self;
    fn to_value(&self) -> ParamValue;
    fn assign(&mut self, kind: &ParamKind, value: ParamValue) -> Result<(), ParamError>;
}

impl ParamField for f32 {
    fn default_for(kind: &ParamKind) -> Self {
        match *kind {
            ParamKind::Float { default, .. } => default,
            _ => 0.0,
        }
    }

    fn to_value(&self) -> ParamValue {
        ParamValue::Float(*self)
    }

    fn assign(&mut self, kind: &ParamKind, value: ParamValue) -> Result<(), ParamError> {
        match (*kind, value) {
            (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) => {
                if !v.is_finite() {
                    return Err(ParamError::Invalid);
                }
                *self = v.clamp(min, max);
                Ok(())
            }
            _ => Err(ParamError::TypeMismatch),
        }
    }
}

impl ParamField for bool {
    fn default_for(kind: &ParamKind) -> Self {
        match *kind {
            ParamKind::Toggle { default } => default,
            _ => false,
        }
    }

    fn to_value(&self) -> ParamValue {
        ParamValue::Bool(*self)
    }

    fn assign(&mut self, kind: &ParamKind, value: ParamValue) -> Result<(), ParamError> {
        match (*kind, value) {
            (ParamKind::Toggle { .. }, ParamValue::Bool(b)) => {
                *self = b;
                Ok(())
            }
            _ => Err(ParamError::TypeMismatch),
        }
    }
}

impl ParamField for u8 {
    fn default_for(kind: &ParamKind) -> Self {
        match *kind {
            ParamKind::Choice { default, .. } => default,
            _ => 0,
        }
    }

    fn to_value(&self) -> ParamValue {
        ParamValue::Choice(*self)
    }

    fn assign(&mut self, kind: &ParamKind, value: ParamValue) -> Result<(), ParamError> {
        match (*kind, value) {
            (ParamKind::Choice { options, .. }, ParamValue::Choice(i)) => {
                if (i as usize) >= options.len() {
                    return Err(ParamError::Invalid);
                }
                *self = i;
                Ok(())
            }
            _ => Err(ParamError::TypeMismatch),
        }
    }
}

/// Compile-time composition of schemas, used by `#[derive(Params)]` and
/// `#[derive(Chain)]` to build one flat schema from nested ones.
///
/// Nested ids get their field's name as a prefix (`"amp.drive"`). The
/// prefixed strings are built in a `const` byte buffer, so the result is
/// still a `&'static [ParamDesc]` with no runtime work.
///
/// Two parameters with the same id are a compile error:
///
/// ```compile_fail
/// use euterpe_core::{ParamKind, ParamSet, Params};
///
/// #[derive(Params, Clone)]
/// struct Clash {
///     #[param(id = "x", label = "A", kind = ParamKind::toggle(false))]
///     a: bool,
///     #[param(id = "x", label = "B", kind = ParamKind::toggle(false))]
///     b: bool,
/// }
/// let _ = Clash::SCHEMA;
/// ```
pub mod compose {
    use super::{ParamDesc, ParamKind};

    /// One piece of a composed schema: `schema`'s ids, each prefixed with
    /// `prefix` and a dot (no prefix when `prefix` is empty).
    pub struct Part {
        pub prefix: &'static str,
        pub schema: &'static [ParamDesc],
    }

    const fn prefix_len(prefix: &str) -> usize {
        if prefix.is_empty() {
            0
        } else {
            prefix.len() + 1
        }
    }

    /// Parameters in all parts.
    pub const fn count(parts: &[Part]) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < parts.len() {
            n += parts[i].schema.len();
            i += 1;
        }
        n
    }

    /// Bytes of all prefixed ids together.
    pub const fn ids_len(parts: &[Part]) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < parts.len() {
            let mut j = 0;
            while j < parts[i].schema.len() {
                n += prefix_len(parts[i].prefix) + parts[i].schema[j].id.len();
                j += 1;
            }
            i += 1;
        }
        n
    }

    const fn copy(dst: &mut [u8], at: usize, src: &[u8]) -> usize {
        let mut k = 0;
        while k < src.len() {
            dst[at + k] = src[k];
            k += 1;
        }
        at + k
    }

    /// All prefixed ids, back to back. `T` must be [`ids_len`].
    pub const fn ids<const T: usize>(parts: &[Part]) -> [u8; T] {
        let mut buf = [0u8; T];
        let mut at = 0;
        let mut i = 0;
        while i < parts.len() {
            let mut j = 0;
            while j < parts[i].schema.len() {
                if !parts[i].prefix.is_empty() {
                    at = copy(&mut buf, at, parts[i].prefix.as_bytes());
                    at = copy(&mut buf, at, b".");
                }
                at = copy(&mut buf, at, parts[i].schema[j].id.as_bytes());
                j += 1;
            }
            i += 1;
        }
        assert!(at == T, "ids buffer has the wrong length");
        buf
    }

    const fn eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut k = 0;
        while k < a.len() {
            if a[k] != b[k] {
                return false;
            }
            k += 1;
        }
        true
    }

    /// The composed schema, ids pointing into `ids`. `L` must be [`count`].
    /// Fails to compile if two parameters end up with the same id.
    pub const fn descs<const L: usize>(parts: &[Part], ids: &'static [u8]) -> [ParamDesc; L] {
        let mut out = [ParamDesc {
            id: "",
            label: "",
            kind: ParamKind::toggle(false),
        }; L];
        let mut rest = ids;
        let mut n = 0;
        let mut i = 0;
        while i < parts.len() {
            let mut j = 0;
            while j < parts[i].schema.len() {
                let d = parts[i].schema[j];
                let (id, tail) = rest.split_at(prefix_len(parts[i].prefix) + d.id.len());
                rest = tail;
                let mut k = 0;
                while k < n {
                    assert!(!eq(out[k].id.as_bytes(), id), "duplicate parameter id");
                    k += 1;
                }
                let Ok(id) = core::str::from_utf8(id) else {
                    panic!("parameter id is not UTF-8");
                };
                out[n] = ParamDesc {
                    id,
                    label: d.label,
                    kind: d.kind,
                };
                n += 1;
                j += 1;
            }
            i += 1;
        }
        assert!(n == L, "schema has the wrong length");
        out
    }
}

/// Declare a parameter struct and implement [`ParamSet`] for it.
///
/// Each field is `name: type = ("id", "Label", kind)`, where `type` is `f32`
/// (float), `bool` (toggle) or `u8` (choice index) and `kind` is a
/// [`ParamKind`] built with its const constructors. See the module docs.
#[macro_export]
macro_rules! params {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$fmeta:meta])*
                $field:ident : $ty:ty = ($id:literal, $label:literal, $kind:expr)
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        $vis struct $name {
            $( $(#[$fmeta])* pub $field: $ty, )*
        }

        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self {
                    $( $field: <$ty as $crate::params::ParamField>::default_for(&$kind), )*
                }
            }
        }

        impl $crate::params::ParamSet for $name {
            const SCHEMA: &'static [$crate::params::ParamDesc] = &[
                $( $crate::params::ParamDesc { id: $id, label: $label, kind: $kind }, )*
            ];

            fn get(&self, id: &str) -> ::core::option::Option<$crate::params::ParamValue> {
                match id {
                    $( $id => ::core::option::Option::Some(
                        <$ty as $crate::params::ParamField>::to_value(&self.$field)
                    ), )*
                    _ => ::core::option::Option::None,
                }
            }

            fn set(
                &mut self,
                id: &str,
                value: $crate::params::ParamValue,
            ) -> ::core::result::Result<(), $crate::params::ParamError> {
                match id {
                    $( $id => <$ty as $crate::params::ParamField>::assign(&mut self.$field, &$kind, value), )*
                    _ => ::core::result::Result::Err($crate::params::ParamError::UnknownId),
                }
            }
        }
    };
}

/// Proofs (`cargo kani -p euterpe_core`).
#[cfg(kani)]
mod proofs {
    use super::*;

    const OPTIONS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

    /// A pot reading, any f32 at all (NaN, infinities, out of range), maps
    /// to a valid choice index, which `set` then accepts.
    #[kani::proof]
    fn any_normalized_reading_is_a_valid_choice() {
        let len: usize = kani::any();
        kani::assume((1..=OPTIONS.len()).contains(&len));
        let kind = ParamKind::choice(&OPTIONS[..len], 0);
        let ParamValue::Choice(i) = kind.from_normalized(kani::any()) else {
            panic!("a choice maps to a choice");
        };
        assert!(usize::from(i) < len);
        let mut field = 0u8;
        assert!(field.assign(&kind, ParamValue::Choice(i)).is_ok());
    }

    /// Whatever value arrives, a float field ends up inside its range or
    /// the value is rejected; it is never left out of range or NaN.
    #[kani::proof]
    fn float_fields_stay_in_range() {
        let kind = ParamKind::float(-24.0, 6.0, 0.0);
        let mut field = 0.0f32;
        let v: f32 = kani::any();
        let ok = field.assign(&kind, ParamValue::Float(v)).is_ok();
        assert!((-24.0..=6.0).contains(&field));
        assert!(ok == v.is_finite());
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact, bit-identical results on purpose"
)]
mod tests {
    use super::*;

    const MODES: &[&str] = &["Clean", "Crunch", "Lead"];

    crate::params! {
        struct TestParams {
            gain: f32 = ("gain", "Gain", ParamKind::float(0.0, 10.0, 5.0)),
            freq: f32 = ("freq", "Freq", ParamKind::float(20.0, 20_000.0, 1000.0).unit("Hz").taper(Taper::Exponential)),
            on: bool = ("on", "On", ParamKind::toggle(true)),
            mode: u8 = ("mode", "Mode", ParamKind::choice(MODES, 1)),
        }
    }

    #[test]
    fn defaults_come_from_schema() {
        let p = TestParams::default();
        assert_eq!(p.gain, 5.0);
        assert_eq!(p.freq, 1000.0);
        assert!(p.on);
        assert_eq!(p.mode, 1);
        for d in TestParams::SCHEMA {
            assert_eq!(p.get(d.id), Some(d.kind.default_value()));
        }
    }

    #[test]
    fn set_clamps_and_validates() {
        let mut p = TestParams::default();
        p.set("gain", ParamValue::Float(99.0)).unwrap();
        assert_eq!(p.gain, 10.0);
        assert_eq!(
            p.set("gain", ParamValue::Float(f32::NAN)),
            Err(ParamError::Invalid)
        );
        assert_eq!(
            p.set("gain", ParamValue::Bool(true)),
            Err(ParamError::TypeMismatch)
        );
        assert_eq!(
            p.set("mode", ParamValue::Choice(3)),
            Err(ParamError::Invalid)
        );
        assert_eq!(
            p.set("nope", ParamValue::Float(0.0)),
            Err(ParamError::UnknownId)
        );
    }

    #[test]
    fn parse_text() {
        let mut p = TestParams::default();
        p.set_str("on", "off").unwrap();
        assert!(!p.on);
        p.set_str("mode", "lead").unwrap();
        assert_eq!(p.mode, 2);
        p.set_str("mode", "0").unwrap();
        assert_eq!(p.mode, 0);
        p.set_str("gain", " 2.5 ").unwrap();
        assert_eq!(p.gain, 2.5);
        assert_eq!(p.set_str("gain", "loud"), Err(ParamError::Parse));
    }

    #[test]
    fn normalized_round_trip() {
        let kind = TestParams::desc("freq").unwrap().kind;
        // Exponential taper: the midpoint of 20..20k is the geometric mean, ~632 Hz.
        let ParamValue::Float(mid) = kind.from_normalized(0.5) else {
            panic!()
        };
        assert!((mid - 632.46).abs() < 0.1, "{mid}");
        for n in [0.0, 0.1, 0.37, 0.5, 0.9, 1.0] {
            let back = kind.to_normalized(kind.from_normalized(n));
            assert!((back - n).abs() < 1e-5, "{n} -> {back}");
        }
        let mode = TestParams::desc("mode").unwrap().kind;
        assert_eq!(mode.from_normalized(0.49), ParamValue::Choice(1));
        assert_eq!(mode.from_normalized(1.0), ParamValue::Choice(2));
    }
}
