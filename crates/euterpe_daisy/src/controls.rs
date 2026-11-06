//! The Seed's panel inputs: pots on the ADC pins, switches on GPIO.
//!
//! | Panel index | Seed pin |
//! | --- | --- |
//! | pot 0–7 | A0–A7 (D15–D22) |
//! | switch / button 0–7 | D7–D14, to ground (internal pull-ups) |
//!
//! Wire each pot as a divider between 3V3A and AGND with the wiper on its
//! pin. A switch or button closes its pin to DGND. Inputs that no binding
//! uses are not read.

use daisy::hal;
use daisy::pins::Gpio;
use euterpe_core::panel::{Binding, Control, MAX_POTS, MAX_SWITCHES, Readings};
use hal::adc::{Adc, Enabled, Resolution};
use hal::gpio::{Analog, ErasedPin, Input, gpioa, gpiob, gpioc};
use hal::pac::ADC1;

/// Bit `i` set if the panel binds pot `i`.
#[must_use]
pub const fn bound_pots(panel: &[Binding]) -> u8 {
    let mut mask = 0_u8;
    let mut i = 0;
    while i < panel.len() {
        if let Control::Pot(p) = panel[i].control
            && (p as usize) < MAX_POTS
        {
            mask |= 1 << p;
        }
        i += 1;
    }
    mask
}

/// The pot pins, A0 to A7.
struct Pots {
    a0: gpioc::PC0<Analog>,
    a1: gpioa::PA3<Analog>,
    a2: gpiob::PB1<Analog>,
    a3: gpioa::PA7<Analog>,
    a4: gpioa::PA6<Analog>,
    a5: gpioc::PC1<Analog>,
    a6: gpioc::PC4<Analog>,
    a7: gpioa::PA5<Analog>,
}

/// The ADC, the pot pins and the switch pins.
pub struct Controls {
    adc: Adc<ADC1, Enabled>,
    pots: Pots,
    switches: [ErasedPin<Input>; MAX_SWITCHES],
    /// Bit `i`: pot `i` is read.
    read_pots: u8,
}

/// One blocking 16-bit conversion, scaled to `0.0..=1.0`.
macro_rules! convert {
    ($adc:expr, $pin:expr) => {{
        $adc.start_conversion($pin);
        let counts = loop {
            if let Ok(c) = $adc.read_sample() {
                break c;
            }
        };
        #[expect(clippy::cast_precision_loss, reason = "16-bit counts are exact in f32")]
        let x = counts as f32 / 65_535.0;
        x
    }};
}

impl Controls {
    /// Take the panel pins from `gpio` and the enabled `adc`, reading only
    /// the pots in `read_pots` (see [`bound_pots`]).
    pub fn new(mut adc: Adc<ADC1, Enabled>, gpio: Gpio, read_pots: u8) -> Self {
        adc.set_resolution(Resolution::SixteenBit);
        Self {
            adc,
            pots: Pots {
                a0: gpio.PIN_15,
                a1: gpio.PIN_16,
                a2: gpio.PIN_17,
                a3: gpio.PIN_18,
                a4: gpio.PIN_19,
                a5: gpio.PIN_20,
                a6: gpio.PIN_21,
                a7: gpio.PIN_22,
            },
            switches: [
                gpio.PIN_7.into_pull_up_input().erase(),
                gpio.PIN_8.into_pull_up_input().erase(),
                gpio.PIN_9.into_pull_up_input().erase(),
                gpio.PIN_10.into_pull_up_input().erase(),
                gpio.PIN_11.into_pull_up_input().erase(),
                gpio.PIN_12.into_pull_up_input().erase(),
                gpio.PIN_13.into_pull_up_input().erase(),
                gpio.PIN_14.into_pull_up_input().erase(),
            ],
            read_pots,
        }
    }

    /// Read the bound pots and every switch. A few microseconds per pot.
    pub fn read(&mut self, into: &mut Readings) {
        let (adc, p, mask) = (&mut self.adc, &mut self.pots, self.read_pots);
        let r = &mut into.pots;
        macro_rules! read_bound {
            ($($i:literal => $pin:ident),*) => {$(
                if mask & (1_u8 << $i) != 0 {
                    r[$i] = convert!(adc, &mut p.$pin);
                }
            )*};
        }
        read_bound!(0_usize => a0, 1_usize => a1, 2_usize => a2, 3_usize => a3, 4_usize => a4, 5_usize => a5, 6_usize => a6, 7_usize => a7);
        // Closed pulls the pin low.
        into.switches = self
            .switches
            .iter()
            .enumerate()
            .fold(0, |bits, (i, pin)| bits | (u8::from(pin.is_low()) << i));
    }
}
