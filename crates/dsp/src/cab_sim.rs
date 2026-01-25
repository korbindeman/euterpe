use crate::Process;
use crate::params::Toggle;
use microfft::Complex32;

/// Block size for FFT processing (input samples per block).
const BLOCK_SIZE: usize = 256;
/// FFT size (2x block size for linear convolution via overlap-add).
const FFT_SIZE: usize = 512;
/// Number of IR partitions for 2048-sample IR support.
const NUM_PARTITIONS: usize = 8;
/// Maximum IR length supported (NUM_PARTITIONS × BLOCK_SIZE).
pub const IR_MAX_LENGTH: usize = NUM_PARTITIONS * BLOCK_SIZE;

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

pub struct CabSim {
    params: CabSimParams,
    /// Input buffer (accumulate samples until block is ready)
    input_buffer: [f32; BLOCK_SIZE],
    input_pos: usize,
    /// Output buffer (drain samples after processing)
    output_buffer: [f32; BLOCK_SIZE],
    output_pos: usize,
    /// Pre-computed IR partitions in frequency domain
    ir_partitions: [[Complex32; FFT_SIZE]; NUM_PARTITIONS],
    /// Frequency-domain delay line for partitioned convolution
    fdl: [[Complex32; FFT_SIZE]; NUM_PARTITIONS],
    fdl_pos: usize,
    /// Overlap buffer for overlap-add
    overlap: [f32; BLOCK_SIZE],
    /// Whether we have a valid IR loaded
    ir_loaded: bool,
}

impl CabSim {
    pub fn new() -> Self {
        Self {
            params: CabSimParams::new(false),
            input_buffer: [0.0; BLOCK_SIZE],
            input_pos: 0,
            output_buffer: [0.0; BLOCK_SIZE],
            output_pos: 0,
            ir_partitions: [[Complex32 { re: 0.0, im: 0.0 }; FFT_SIZE]; NUM_PARTITIONS],
            fdl: [[Complex32 { re: 0.0, im: 0.0 }; FFT_SIZE]; NUM_PARTITIONS],
            fdl_pos: 0,
            overlap: [0.0; BLOCK_SIZE],
            ir_loaded: false,
        }
    }

    pub fn params(&self) -> &CabSimParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut CabSimParams {
        &mut self.params
    }

    /// Load an impulse response. The IR is split into partitions and each
    /// partition is FFT'd for frequency-domain convolution.
    pub fn set_ir(&mut self, ir: &[f32]) {
        // Clear all partitions
        for partition in &mut self.ir_partitions {
            for c in partition.iter_mut() {
                *c = Complex32 { re: 0.0, im: 0.0 };
            }
        }

        // Split IR into partitions and FFT each
        let mut partition_idx = 0;
        let mut ir_pos = 0;

        while partition_idx < NUM_PARTITIONS && ir_pos < ir.len() {
            // Prepare time-domain buffer: IR chunk zero-padded to FFT_SIZE
            let mut time_buf = [Complex32 { re: 0.0, im: 0.0 }; FFT_SIZE];

            let chunk_len = (ir.len() - ir_pos).min(BLOCK_SIZE);
            for i in 0..chunk_len {
                time_buf[i].re = ir[ir_pos + i];
            }

            // In-place FFT
            let _ = microfft::complex::cfft_512(&mut time_buf);

            // Store the spectrum
            self.ir_partitions[partition_idx] = time_buf;

            partition_idx += 1;
            ir_pos += BLOCK_SIZE;
        }

        // Clear FDL and overlap buffer
        for fdl_part in &mut self.fdl {
            for c in fdl_part.iter_mut() {
                *c = Complex32 { re: 0.0, im: 0.0 };
            }
        }
        self.fdl_pos = 0;
        self.overlap = [0.0; BLOCK_SIZE];

        // Reset buffers
        self.input_buffer = [0.0; BLOCK_SIZE];
        self.input_pos = 0;
        self.output_buffer = [0.0; BLOCK_SIZE];
        self.output_pos = 0;

        self.ir_loaded = !ir.is_empty();
    }

    /// Process a full block using partitioned overlap-add convolution.
    fn process_block(&mut self) {
        // Prepare input spectrum: input samples zero-padded to FFT_SIZE
        let mut input_spectrum = [Complex32 { re: 0.0, im: 0.0 }; FFT_SIZE];
        for i in 0..BLOCK_SIZE {
            input_spectrum[i].re = self.input_buffer[i];
        }

        // Forward FFT of input block
        let _ = microfft::complex::cfft_512(&mut input_spectrum);

        // Store in frequency-domain delay line
        self.fdl[self.fdl_pos] = input_spectrum;

        // Accumulate frequency-domain products across all partitions
        let mut accum = [Complex32 { re: 0.0, im: 0.0 }; FFT_SIZE];

        for i in 0..NUM_PARTITIONS {
            // Index into FDL: current position minus partition index (circular)
            let fdl_idx = (self.fdl_pos + NUM_PARTITIONS - i) % NUM_PARTITIONS;

            // Complex multiply and accumulate
            for j in 0..FFT_SIZE {
                let a = self.fdl[fdl_idx][j];
                let b = self.ir_partitions[i][j];
                // (a.re + a.im*i) * (b.re + b.im*i) = (a.re*b.re - a.im*b.im) + (a.re*b.im + a.im*b.re)*i
                accum[j].re += a.re * b.re - a.im * b.im;
                accum[j].im += a.re * b.im + a.im * b.re;
            }
        }

        // Inverse FFT using conjugate trick: IFFT(X) = conj(FFT(conj(X))) / N
        for c in &mut accum {
            c.im = -c.im; // conjugate
        }
        let _ = microfft::complex::cfft_512(&mut accum);
        let scale = 1.0 / FFT_SIZE as f32;
        for c in &mut accum {
            c.re *= scale;
            c.im = -c.im * scale; // conjugate and scale
        }

        // Overlap-add: first BLOCK_SIZE samples go to output, rest to overlap
        for i in 0..BLOCK_SIZE {
            self.output_buffer[i] = accum[i].re + self.overlap[i];
        }
        for i in 0..BLOCK_SIZE {
            self.overlap[i] = accum[BLOCK_SIZE + i].re;
        }

        // Advance FDL position
        self.fdl_pos = (self.fdl_pos + 1) % NUM_PARTITIONS;
    }
}

impl Process for CabSim {
    fn process(&mut self, sample: f32) -> f32 {
        // Clear dirty flag
        let _ = self.params.enabled.take_dirty();

        // Bypass if disabled or no IR loaded
        if !self.params.enabled.get() || !self.ir_loaded {
            return sample;
        }

        // Store input sample
        self.input_buffer[self.input_pos] = sample;

        // Get output sample from previous block
        let output = self.output_buffer[self.output_pos];

        self.input_pos += 1;
        self.output_pos += 1;

        // Process block when input buffer is full
        if self.input_pos >= BLOCK_SIZE {
            self.process_block();
            self.input_pos = 0;
            self.output_pos = 0;
        }

        output
    }
}
