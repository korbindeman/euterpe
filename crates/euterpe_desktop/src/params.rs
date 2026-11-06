//! Parameter values shared between the UI and the audio thread.

use std::sync::atomic::{AtomicU32, Ordering};

use euterpe_core::{ParamDesc, ParamKind, ParamSet, ParamValue};

/// One atomic slot per parameter in a schema. The UI writes, the audio
/// thread reads every block. No locks, no allocation after construction.
pub struct ParamBank {
    schema: &'static [ParamDesc],
    slots: Vec<AtomicU32>,
}

impl ParamBank {
    /// A bank holding every parameter's default.
    pub fn new(schema: &'static [ParamDesc]) -> Self {
        let slots = schema
            .iter()
            .map(|d| AtomicU32::new(encode(d.kind.default_value())))
            .collect();
        Self { schema, slots }
    }

    pub fn schema(&self) -> &'static [ParamDesc] {
        self.schema
    }

    /// The current value of parameter `index`.
    pub fn get(&self, index: usize) -> ParamValue {
        decode(
            &self.schema[index].kind,
            self.slots[index].load(Ordering::Relaxed),
        )
    }

    /// Set parameter `index`. Floats are clamped to their range.
    pub fn set(&self, index: usize, value: ParamValue) {
        let value = match (self.schema[index].kind, value) {
            (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) if !v.is_nan() => {
                ParamValue::Float(v.clamp(min, max))
            }
            (ParamKind::Float { .. }, ParamValue::Float(_)) => return,
            (_, v) => v,
        };
        self.slots[index].store(encode(value), Ordering::Relaxed);
    }

    /// Reset every parameter to its default.
    pub fn reset(&self) {
        for (i, d) in self.schema.iter().enumerate() {
            self.set(i, d.kind.default_value());
        }
    }

    /// Copy every value into `params`. Called on the audio thread.
    pub fn apply<P: ParamSet>(&self, params: &mut P) {
        for (i, d) in self.schema.iter().enumerate() {
            // Ids come from the same schema, so this cannot fail.
            let _ = params.set(d.id, self.get(i));
        }
    }
}

fn encode(value: ParamValue) -> u32 {
    match value {
        ParamValue::Float(v) => v.to_bits(),
        ParamValue::Bool(b) => u32::from(b),
        ParamValue::Choice(i) => u32::from(i),
    }
}

fn decode(kind: &ParamKind, bits: u32) -> ParamValue {
    match kind {
        ParamKind::Float { .. } => ParamValue::Float(f32::from_bits(bits)),
        ParamKind::Toggle { .. } => ParamValue::Bool(bits != 0),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "only u8 values are ever encoded"
        )]
        ParamKind::Choice { .. } => ParamValue::Choice(bits as u8),
    }
}

#[cfg(test)]
mod tests {
    use euterpe_core::params;

    use super::*;

    params! {
        pub struct Ps {
            level: f32 = ("level", "Level", ParamKind::float(-12.0, 12.0, 0.0)),
            on: bool = ("on", "On", ParamKind::toggle(true)),
            mode: u8 = ("mode", "Mode", ParamKind::choice(&["a", "b", "c"], 1)),
        }
    }

    #[test]
    fn starts_at_defaults_and_round_trips() {
        let bank = ParamBank::new(Ps::SCHEMA);
        assert_eq!(bank.get(0), ParamValue::Float(0.0));
        assert_eq!(bank.get(1), ParamValue::Bool(true));
        assert_eq!(bank.get(2), ParamValue::Choice(1));
        bank.set(0, ParamValue::Float(-3.5));
        bank.set(1, ParamValue::Bool(false));
        bank.set(2, ParamValue::Choice(2));
        let mut p = Ps::default();
        bank.apply(&mut p);
        assert_eq!(p.get("level"), Some(ParamValue::Float(-3.5)));
        assert_eq!(p.get("on"), Some(ParamValue::Bool(false)));
        assert_eq!(p.get("mode"), Some(ParamValue::Choice(2)));
    }

    #[test]
    fn clamps_floats_and_ignores_nan() {
        let bank = ParamBank::new(Ps::SCHEMA);
        bank.set(0, ParamValue::Float(100.0));
        assert_eq!(bank.get(0), ParamValue::Float(12.0));
        bank.set(0, ParamValue::Float(f32::NAN));
        assert_eq!(bank.get(0), ParamValue::Float(12.0));
        bank.reset();
        assert_eq!(bank.get(0), ParamValue::Float(0.0));
    }
}
