#![no_main]
#![no_std]

mod ir;

use core::cell::RefCell;

use daisy_runtime::cortex_m::{self, asm, interrupt::Mutex};
use daisy_runtime::cortex_m_rt::entry;
use daisy_runtime::pac::{self, interrupt};
use daisy_runtime::{audio, daisy, panic_halt as _};

use preamp::channel::Channel;
use preamp::dsp::Process;
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

    // Heartbeat LED.
    let one_second = ccdr.clocks.sys_ck().to_Hz();
    loop {
        led_user.toggle();
        asm::delay(one_second);
        led_user.toggle();
        asm::delay(one_second);
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
