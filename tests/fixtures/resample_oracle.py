"""The resampling arithmetic a decoded file goes through when it is loaded at another rate than the
project's, written out again in plain Python, f64 throughout, so known answers
for resampled sounds come from here and never from Dawsome.

The resampler is rubato 5's synchronous FFT resampler with a fixed input size, as
crates/media/src/resample.rs builds it (chunk 1024 frames, sub-chunks of about 256):
  block sizes   g = gcd(in, out); n = ceil((1024 // 4) / (in // g)) blocks of the reduced ratio:
                n_in = n * in // g, n_out = n * out // g
  cutoff        c(N) = 1 / (k1/N + k2/N^2 + k3/N^3 + 1) with the squared Blackman-Harris fit
                (k1, k2, k3) = (13.745202940783823, 121.73532586374934, 5964.163279612051);
                c(n_in) when upsampling, c(n_out) * n_out / n_in when downsampling
  filter        h[x] = w(x)^2 * sinc((x - n_in//2) * cutoff), x in 0..n_in, w the periodic
                Blackman-Harris window (0.35875, 0.48829, 0.14128, 0.01168), normalized to
                sum 1, then divided by 2 * n_in (the FFT's scale)
  each block    of n_in input frames: y = block (*) h (linear convolution, 2 * n_in long),
                Y = DFT_{2 n_in}(y); the first min(n_in + 1, n_out) bins are kept (the rest of the
                n_out + 1 output bins are 0) and z = real inverse DFT_{2 n_out}, unnormalized;
                z[0:n_out] plus the previous block's z[n_out:] is the block's output
  whole file    blocks follow each other from input frame 0 (the last one padded with zeros);
                the first n_out // 2 output frames (the delay) are dropped, and the output is
                cut (or padded with zeros) to exactly ceil(frames * out / in) frames
Usable as a module: load it by path (importlib) from the generators beside it."""
import cmath, math


def block_sizes(rate_in, rate_out, chunk=1024):
    g = math.gcd(rate_in, rate_out)
    sub = max(chunk // 256, 1)
    wanted = chunk // sub
    blocks = max(-(-wanted // (rate_in // g)), 1)
    return blocks * rate_in // g, blocks * rate_out // g


def cutoff(n_in, n_out):
    k1, k2, k3 = 13.745202940783823, 121.73532586374934, 5964.163279612051
    fit = lambda n: 1.0 / (k1 / n + k2 / n ** 2 + k3 / n ** 3 + 1.0)
    return fit(n_out) * n_out / n_in if n_in > n_out else fit(n_in)


def sinc(x):
    return 1.0 if x == 0 else math.sin(math.pi * x) / (math.pi * x)


def filter_taps(n_in, fc):
    def window(x):
        a = 2.0 * math.pi * x / n_in
        return 0.35875 - 0.48829 * math.cos(a) + 0.14128 * math.cos(2 * a) - 0.01168 * math.cos(3 * a)
    y = [window(x) ** 2 * sinc((x - n_in // 2) * fc) for x in range(n_in)]
    total = sum(y)
    return [v / total / (2 * n_in) for v in y]


def resample_unit(block, taps, n_in, n_out):
    """One block's output, 2 * n_out frames long (the second half overlaps the next block)."""
    y = [0.0] * (2 * n_in)
    for i, x in enumerate(block):
        if x:
            for j, t in enumerate(taps):
                y[i + j] += x * t
    kept = min(n_in + 1, n_out)
    spectrum = []
    for k in range(kept):
        turn = -2j * math.pi * k / (2 * n_in)
        spectrum.append(sum(v * cmath.exp(turn * t) for t, v in enumerate(y) if v))
    out = []
    for m in range(2 * n_out):
        value = spectrum[0].real
        for k in range(1, kept):
            # Every bin below the output's Nyquist bin counts twice (it has a mirror image).
            value += 2.0 * (spectrum[k] * cmath.exp(2j * math.pi * k * m / (2 * n_out))).real
        out.append(value)
    return out


def resample(samples, rate_in, rate_out):
    """`samples` (mono, at `rate_in`) as they load at `rate_out`."""
    if rate_in == rate_out or not samples:
        return list(samples)
    n_in, n_out = block_sizes(rate_in, rate_out)
    taps = filter_taps(n_in, cutoff(n_in, n_out))
    frames = len(samples)
    want = -(-frames * rate_out // rate_in)
    delay = n_out // 2
    blocks = -(-(want + delay) * n_in // n_out) // n_in + 1
    padded = list(samples) + [0.0] * (blocks * n_in - frames)
    output, overlap = [], [0.0] * n_out
    for b in range(blocks):
        z = resample_unit(padded[b * n_in:(b + 1) * n_in], taps, n_in, n_out)
        output.extend(z[m] + overlap[m] for m in range(n_out))
        overlap = z[n_out:]
    output = output[delay:delay + want]
    return output + [0.0] * (want - len(output))


if __name__ == "__main__":
    print("44100 -> 48000 blocks", block_sizes(44100, 48000), "impulse loads as", resample([1.0], 44100, 48000))
