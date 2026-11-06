"""Design the half-band FIR stages used by euterpe_dsp::oversample.

Kaiser-windowed half-band sinc. Prints the nonzero even taps as Rust arrays
plus the measured passband ripple and stopband attenuation. numpy only.

    python3 tools/design/halfband.py

Every stage is designed for a base rate of 48 kHz and a 20 kHz audio band
(passband edge 0.4167 fs). Stage k runs at 2^k x the base rate.
"""
import numpy as np

BASE = 48_000.0
BAND = 20_000.0
ATTEN = 100.0  # dB target

def kaiser_beta(a):
    return 0.1102 * (a - 8.7) if a > 50 else 0.5842 * (a - 21) ** 0.4 + 0.07886 * (a - 21)

def design(m, beta):
    L = 4 * m + 3
    c = 2 * m + 1
    n = np.arange(L) - c
    h = 0.5 * np.sinc(n / 2.0) * np.kaiser(L, beta)
    h[c] = 0.5
    for k in range(L):
        if k % 2 == 1 and k != c:
            h[k] = 0.0
    # Normalise DC gain to exactly 1 (even taps sum to 0.5).
    even = h[0::2]
    even *= 0.5 / even.sum()
    h[0::2] = even
    return h

def measure(h, rate, pass_hz, stop_hz):
    nfft = 1 << 18
    H = np.abs(np.fft.rfft(h, nfft))
    f = np.arange(len(H)) * rate / nfft
    pb = H[f <= pass_hz]
    sb = H[f >= stop_hz]
    ripple = 20 * np.log10(pb.max() / pb.min())
    atten = -20 * np.log10(sb.max())
    return ripple, atten

stages = []
for k, name in [(1, "STEEP"), (2, "MEDIUM"), (3, "GENTLE")]:
    rate = BASE * 2 ** k          # rate of the high side of this stage
    low = rate / 2                # rate of the low side
    stop = low - BAND             # images/aliases of the audio band start here
    best = None
    for m in range(1, 40):
        for a in np.arange(60, 140, 1.0):
            h = design(m, kaiser_beta(a))
            r, at = measure(h, rate, BAND, stop)
            if at >= ATTEN and r < 0.001:
                best = (m, a, h, r, at)
                break
        if best:
            break
    m, a, h, r, at = best
    stages.append((name, k, m, h, r, at, stop))

for name, k, m, h, r, at, stop in stages:
    even = h[0::2]
    n = len(even)
    print(f"// {name}: x{2**k // 2} -> x{2**k}, {4*m+3} taps ({n} nonzero + centre), "
          f"stopband from {stop/1000:.0f} kHz: {at:.1f} dB, passband ripple {r:.5f} dB")
    print(f"pub static {name}: [f32; {n}] = [")
    for i in range(0, n, 4):
        print("    " + " ".join(np.format_float_scientific(np.float32(v), unique=True, exp_digits=2) + "," for v in even[i:i+4]))
    print("];")
