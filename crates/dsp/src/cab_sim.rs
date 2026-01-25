use crate::Process;
use crate::params::Toggle;

pub struct CabSimParams {
    pub enabled: Toggle,
}

impl CabSimParams {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: Toggle::new("Cab Sim", enabled),
        }
    }
}

pub struct CabSim<const N: usize> {
    params: CabSimParams,
    ir: [f32; N],
    buffer: [f32; N],
    write_pos: usize,
}

impl<const N: usize> CabSim<N> {
    pub fn new() -> Self {
        // Default IR is a unit impulse (passthrough) so that enabling
        // the cab sim before loading an IR doesn't produce silence.
        let mut ir = [0.0; N];
        ir[0] = 1.0;
        Self {
            params: CabSimParams::new(false),
            ir,
            buffer: [0.0; N],
            write_pos: 0,
        }
    }

    pub fn params(&self) -> &CabSimParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut CabSimParams {
        &mut self.params
    }

    pub fn set_ir(&mut self, ir: &[f32]) {
        let len = if ir.len() < N { ir.len() } else { N };
        self.ir[..len].copy_from_slice(&ir[..len]);
        // Zero-pad the remainder if the source is shorter than N.
        for sample in &mut self.ir[len..] {
            *sample = 0.0;
        }
    }
}

impl<const N: usize> Process for CabSim<N> {
    fn process(&mut self, sample: f32) -> f32 {
        // Clear dirty flag (no coefficients to update)
        let _ = self.params.enabled.take_dirty();

        if !self.params.enabled.get() {
            return sample;
        }

        self.buffer[self.write_pos] = sample;

        let mut output = 0.0;
        for i in 0..N {
            let buf_idx = (self.write_pos + N - i) % N;
            output += self.ir[i] * self.buffer[buf_idx];
        }

        self.write_pos = (self.write_pos + 1) % N;
        output
    }
}
