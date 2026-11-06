//! Block rendering of a processor over a buffer.

use euterpe_core::{BLOCK, Block, Ctx, Processor};

use crate::fpu;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    pub sample_rate: u32,
    /// Extra output samples after the input ends (reverb and filter tails).
    pub tail: usize,
    /// Drop the processor's reported latency from the start of the output,
    /// so output sample `n` lines up with input sample `n`.
    pub compensate_latency: bool,
}

impl RenderOptions {
    pub const fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            tail: 0,
            compensate_latency: true,
        }
    }
}

/// Render `input` through `processor` with fixed `params`.
///
/// Prepares and resets the processor first, so renders are deterministic.
/// The output is `input.len() + opts.tail` samples long.
pub fn render<P: Processor>(
    processor: &mut P,
    params: &P::Params,
    input: &[f32],
    opts: &RenderOptions,
) -> Vec<f32> {
    let mut params = params.clone();
    render_with(processor, &mut params, input, opts, |_, _| {})
}

/// Like [`render`], but calls `automate(sample_index, &mut params)` before
/// every block, so parameters can change over time. `sample_index` is the
/// input position of the block's first sample.
pub fn render_with<P: Processor>(
    processor: &mut P,
    params: &mut P::Params,
    input: &[f32],
    opts: &RenderOptions,
    mut automate: impl FnMut(usize, &mut P::Params),
) -> Vec<f32> {
    fpu::enable_flush_to_zero();
    processor.prepare(&Ctx::new(opts.sample_rate as f32));
    processor.reset();

    let skip = if opts.compensate_latency {
        processor.latency()
    } else {
        0
    };
    let out_len = input.len() + opts.tail;
    let total = out_len + skip;

    let mut out = Vec::with_capacity(total.next_multiple_of(BLOCK));
    let mut block: Block = [0.0; BLOCK];
    let mut pos = 0;
    while pos < total {
        let n = input.len().saturating_sub(pos).min(BLOCK);
        if n > 0 {
            block[..n].copy_from_slice(&input[pos..pos + n]);
        }
        block[n..].fill(0.0);
        automate(pos, params);
        processor.process(&mut block, params);
        out.extend_from_slice(&block);
        pos += BLOCK;
    }

    out.drain(..skip);
    out.truncate(out_len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Delays its input by a fixed number of samples and reports it.
    #[derive(Default)]
    struct Delay {
        line: Vec<f32>,
        pos: usize,
    }

    const DELAY: usize = 45;

    impl Processor for Delay {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {
            self.line = vec![0.0; DELAY];
        }
        fn reset(&mut self) {
            self.line.fill(0.0);
            self.pos = 0;
        }
        fn process(&mut self, buf: &mut Block, _: &()) {
            for s in buf.iter_mut() {
                let out = self.line[self.pos];
                self.line[self.pos] = *s;
                self.pos = (self.pos + 1) % DELAY;
                *s = out;
            }
        }
        fn latency(&self) -> usize {
            DELAY
        }
    }

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 + 1.0).collect()
    }

    #[test]
    fn compensates_latency_and_keeps_length() {
        let input = ramp(100); // not a multiple of BLOCK
        let out = render(
            &mut Delay::default(),
            &(),
            &input,
            &RenderOptions::new(48_000),
        );
        assert_eq!(out, input);
    }

    #[test]
    fn uncompensated_shows_the_delay() {
        let input = ramp(100);
        let opts = RenderOptions {
            compensate_latency: false,
            tail: DELAY,
            ..RenderOptions::new(48_000)
        };
        let out = render(&mut Delay::default(), &(), &input, &opts);
        assert_eq!(out.len(), 100 + DELAY);
        assert!(out[..DELAY].iter().all(|&s| s == 0.0));
        assert_eq!(&out[DELAY..], &input[..]);
    }

    #[test]
    fn renders_are_deterministic() {
        let input = ramp(77);
        let mut d = Delay::default();
        let opts = RenderOptions::new(48_000);
        assert_eq!(
            render(&mut d, &(), &input, &opts),
            render(&mut d, &(), &input, &opts)
        );
    }
}
