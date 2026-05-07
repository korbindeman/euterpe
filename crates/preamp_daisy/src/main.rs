#![no_main]
#![no_std]

mod ir;

use core::cell::RefCell;

use daisy_runtime::controls::{Button, Pot};
use daisy_runtime::cortex_m::{self, asm, interrupt::Mutex};
use daisy_runtime::cortex_m_rt::entry;
use daisy_runtime::hal::adc;
use daisy_runtime::hal::delay::Delay;
use daisy_runtime::hal::prelude::*;
use daisy_runtime::pac::{self, interrupt};
use daisy_runtime::{audio, daisy, panic_halt as _};

use preamp::audio_app::{ParamModel, ParamValue, apply_hardware_binding};
use preamp::channel::Channel;
use preamp::dsp::Process;
use preamp::model;
use preamp::{ChannelMode, SAMPLE_RATE};

/// Wrapper to allow static UnsafeCell (single-core MCU, so this is safe).
struct SyncStorage(core::cell::UnsafeCell<core::mem::MaybeUninit<Channel>>);
unsafe impl Sync for SyncStorage {}

#[unsafe(link_section = ".sram")]
static CHANNEL_STORAGE: SyncStorage =
    SyncStorage(core::cell::UnsafeCell::new(core::mem::MaybeUninit::uninit()));
static CHANNEL: Mutex<RefCell<Option<&'static mut Channel>>> = Mutex::new(RefCell::new(None));
static AUDIO_INTERFACE: Mutex<RefCell<Option<audio::Interface>>> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    let mut cp = cortex_m::Peripherals::take().unwrap();
    let dp = pac::Peripherals::take().unwrap();

    cp.SCB.enable_icache();
    cp.SCB.enable_dcache(&mut cp.CPUID);

    let board = daisy::Board::take().unwrap();
    let ccdr = daisy::board_freeze_clocks!(board, dp);
    let pins = daisy::board_split_gpios!(board, ccdr, dp);
    let mut led_user = daisy::board_split_leds!(pins).USER;

    // Turn on LED immediately to confirm we got this far.
    led_user.set_high();

    // Set up ADC for volume knob (Daisy Pod Knob 1 = PIN_21).
    let mut delay = Delay::new(cp.SYST, ccdr.clocks);
    let mut adc1 = adc::Adc::adc1(
        dp.ADC1,
        4.MHz(),
        &mut delay,
        ccdr.peripheral.ADC12,
        &ccdr.clocks,
    )
    .enable();
    adc1.set_resolution(adc::Resolution::SixteenBit);
    let mut volume_knob = pins.GPIO.PIN_21.into_analog();

    // Set up buttons (Daisy Pod Button 1 = PIN_27, Button 2 = PIN_28).
    let mode_button = pins.GPIO.PIN_27.into_pull_up_input();
    let comp_button = pins.GPIO.PIN_28.into_pull_up_input();

    // Set up button LEDs (RGB, accent one color each).
    // LED 1 (mode): PIN_20=R, PIN_19=G, PIN_18=B
    // LED 2 (comp): PIN_17=R, PIN_24=G, PIN_23=B
    let mut led1_r = pins.GPIO.PIN_20.into_push_pull_output();
    let mut led2_g = pins.GPIO.PIN_24.into_push_pull_output();
    led1_r.set_low(); // Guitar mode on (active-low)
    led2_g.set_low(); // Compressor on (active-low)

    let audio_interface = daisy::board_split_audio!(ccdr, pins);

    // Set up the DSP channel in Guitar mode with cab sim.
    // Place in SRAM to avoid flash size limits.
    let channel = unsafe {
        let ptr = CHANNEL_STORAGE.0.get();
        (*ptr).write(Channel::new(SAMPLE_RATE))
    };
    channel.set_mode(ChannelMode::Guitar);

    // Load the first available IR.
    if let Some(ir_data) = ir::get_ir(0) {
        channel.set_ir(ir_data);
    }

    // Store globals.
    cortex_m::interrupt::free(|cs| {
        CHANNEL.borrow(cs).replace(Some(channel));
        let ai = audio_interface.spawn().unwrap();
        AUDIO_INTERFACE.borrow(cs).replace(Some(ai));
    });

    // Main loop: read knobs/buttons and update parameters.
    // Poll at ~100Hz for responsive controls, blink LED every ~0.5s.
    let poll_delay = ccdr.clocks.sys_ck().to_Hz() / 100;
    let volume_pot = Pot::new(0.0, 1.0);
    let mut mode_btn = Button::new(3); // ~30ms debounce at 100Hz
    let mut comp_btn = Button::new(3);
    let mut loop_count: u32 = 0;

    loop {
        // Read volume knob.
        let raw: u32 = adc1.read(&mut volume_knob).unwrap();
        let gain = volume_pot.scale(raw);

        // Check buttons.
        let mode_pressed = mode_btn.poll(mode_button.is_low());
        let comp_pressed = comp_btn.poll(comp_button.is_low());

        cortex_m::interrupt::free(|cs| {
            if let Some(channel) = CHANNEL.borrow(cs).borrow_mut().as_mut() {
                let _ = apply_hardware_binding(
                    channel,
                    &model::DAISY_BINDINGS[0],
                    ParamValue::Float(gain),
                );
                let _ = apply_hardware_binding(
                    channel,
                    &model::DAISY_BINDINGS[1],
                    ParamValue::Bool(mode_pressed),
                );
                let _ = apply_hardware_binding(
                    channel,
                    &model::DAISY_BINDINGS[2],
                    ParamValue::Bool(comp_pressed),
                );
            }
        });

        let (current_mode, comp_enabled) = cortex_m::interrupt::free(|cs| {
            if let Some(channel) = CHANNEL.borrow(cs).borrow().as_ref() {
                let current_mode = match channel.get_param(model::param::MODE) {
                    Some(ParamValue::Choice(1)) => ChannelMode::Bass,
                    _ => ChannelMode::Guitar,
                };
                let comp_enabled = match channel.get_param(model::param::COMPRESSOR_ENABLED) {
                    Some(ParamValue::Bool(value)) => value,
                    _ => false,
                };
                (current_mode, comp_enabled)
            } else {
                (ChannelMode::Guitar, false)
            }
        });

        // Update button LEDs (active-low): LED1 red = Guitar mode, LED2 green = compressor on.
        if current_mode == ChannelMode::Guitar {
            led1_r.set_low();
        } else {
            led1_r.set_high();
        }
        if comp_enabled {
            led2_g.set_low();
        } else {
            led2_g.set_high();
        }

        // Heartbeat LED toggles every 50 iterations (~0.5s at 100Hz).
        loop_count = loop_count.wrapping_add(1);
        if loop_count % 50 == 0 {
            led_user.toggle();
        }

        asm::delay(poll_delay);
    }
}

#[interrupt]
fn DMA1_STR1() {
    cortex_m::interrupt::free(|cs| {
        let mut channel_ref = CHANNEL.borrow(cs).borrow_mut();
        let mut audio_ref = AUDIO_INTERFACE.borrow(cs).borrow_mut();

        if let (Some(channel), Some(audio_interface)) = (channel_ref.as_mut(), audio_ref.as_mut()) {
            audio_interface
                .handle_interrupt_dma1_str1(|audio_buffer| {
                    for frame in audio_buffer {
                        let (left, right) = *frame;
                        let mono_in = (left + right) * 0.5;
                        let out = channel.process(mono_in);
                        *frame = (out, out);
                    }
                })
                .unwrap();
        }
    });
}
