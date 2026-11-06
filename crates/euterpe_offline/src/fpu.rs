//! FPU configuration.

/// Flush denormals to zero on the current thread, matching the Daisy's FPU
/// setup so offline renders behave like the hardware. Safe to call repeatedly.
pub fn enable_flush_to_zero() {
    #[cfg(target_arch = "x86_64")]
    {
        // FTZ (bit 15) and DAZ (bit 6) in MXCSR.
        #[allow(
            deprecated,
            reason = "_mm_getcsr/_mm_setcsr are the only stable way to set MXCSR"
        )]
        // SAFETY: SSE is always present on x86_64, and setting FTZ/DAZ only
        // changes how this thread rounds denormals.
        unsafe {
            use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};
            _mm_setcsr(_mm_getcsr() | 0x8040);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // FZ (bit 24) in FPCR.
        // SAFETY: FPCR is readable and writable at EL0 on aarch64, and FZ only
        // changes how this thread rounds denormals.
        unsafe {
            let mut fpcr: u64;
            core::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
            fpcr |= 1 << 24;
            core::arch::asm!("msr fpcr, {}", in(reg) fpcr);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact, bit-identical results on purpose"
)]
mod tests {
    #[test]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn denormals_flush() {
        super::enable_flush_to_zero();
        let tiny = core::hint::black_box(f32::MIN_POSITIVE);
        let half = core::hint::black_box(0.5f32);
        assert_eq!(tiny * half, 0.0);
    }
}
