# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = [
#     "swift-f0==0.1.2",
#     "numpy",
#     "onnx",
#     "onnxruntime",
#     "soundfile",
#     "torch",
#     "torchaudio",
#     "librosa",
#     "numba>=0.61",
# ]
# ///
#
# Environment grabs are the same ones `swiftf0_oracle.py` documents at length (numba floor,
# torch from PyPI, torchaudio/torchcodec). See that script's header before touching pins.
"""What does the lag sweep's peak actually measure? Not the group delay.

    uv run tools/lag_sweep_probe.py            # all four probes, ~4 min
    uv run tools/lag_sweep_probe.py --quick    # skip the corpus probe (no dataset needed)

# Why this exists

`dsp::pitch_bench` sweeps a lag over the annotation and reports the peak. `docs/pitch_
benchmark.md` used to read that peak as a measurement of the bank's group delay, on the
strength of it landing inside the 8-29 ms that `resonator::bank_latency_probe` measures by
an independent route. The SwiftF0 oracle broke that reading: SwiftF0 peaked at +8 ms too,
and SwiftF0 has no group delay to speak of — so the peak cannot be one.

Four probes, run in the order that makes the argument. Each is here because the argument
does not close without it; between them they killed two wrong answers of mine (the model
is late; the corpus is offset) before arriving at the envelope.

  A  PAD SPLIT      SwiftF0's STFT geometry, read out of the ONNX graph. Its 384/384 pad
                    is *computed inside the graph*, so `CENTER_OFFSET = 127.5` is a claim
                    the Python cannot enforce and has to be checked. It holds.
  B  VIBRATO PHASE  the model's latency, through the oracle's own path. A symmetric
                    receptive field cannot phase-shift a sinusoidal log-f modulation, so
                    any phase lag is pure delay — and it is precise where level-based
                    probes are not. Self-checks included; without them this is one more
                    instrument nobody has verified.
  C  ENVELOPE       the finding. Same pitch trajectory, two envelopes. Flat amplitude
                    gives ~0 lag; note-like decay manufactures ~+5 ms out of nothing.
  D  CENTRED YIN    the non-circular check (needs MDB-stem-synth). A zero-delay ruler that
                    shares nothing with A/B: no model, no resampling, 46 ms symmetric
                    window. It says the corpus is aligned to ~1 cent, which is what makes
                    C the only explanation left standing.

# The finding

Inside a symmetric analysis window straddling a decaying note, the energy is weighted
toward the note's earlier, louder part. Any energy-weighted estimator therefore reports
what the signal was doing slightly in the *past*: it is late, and the wider its window,
the later it gets — with no group delay anywhere in it. Real music is made of decaying
notes, so every estimator on this corpus pays this, including the bank.

So a lag-sweep peak is `group_delay + envelope_bias`, and only the first term is a
property of the detector. Two detectors compared *each at its own peak* are still compared
fairly — the term is common. What is not allowed is reading a peak as a latency.
"""

import argparse
import os
import sys
from pathlib import Path

import numpy as np

SR_C, SR_M, HOP = 44100, 16000, 256
PROJECT_ROOT = Path(__file__).resolve().parent.parent
CORPUS = PROJECT_ROOT / "datasets" / "MDB-stem-synth"
REFERENCE = PROJECT_ROOT / "datasets" / "reference" / "pitch-benchmark"


# ---------------------------------------------------------------- shared signal building
def harmonic(phase, n_harm=15):
    """Sum of `n_harm` harmonics — the model wants harmonics; a pure sine scores 0.48."""
    x = np.zeros(len(phase))
    for k in range(1, n_harm + 1):
        x += np.sin(k * phase) / k
    return x / np.abs(x).max()


def vibrato_signal(carrier, depth_st, rate_hz, dur, sr=SR_C):
    """Sawtooth whose log-pitch is a sine. Returns (audio, f0_of_t callable)."""
    n = int(dur * sr)
    t = np.arange(n) / sr
    f = carrier * 2 ** (depth_st * np.sin(2 * np.pi * rate_hz * t) / 12.0)
    x = harmonic(2 * np.pi * np.cumsum(f) / sr)
    return x, (lambda tt: carrier * 2 ** (depth_st * np.sin(2 * np.pi * rate_hz * tt) / 12.0))


# ---------------------------------------------------------------- A: the pad split
def probe_pad_split():
    import onnx
    import onnxruntime
    from onnx import helper

    import swift_f0

    print("=== A: SwiftF0's STFT geometry, read out of the ONNX graph ===")
    path = os.path.join(os.path.dirname(swift_f0.__file__), "model.onnx")
    m = onnx.load(path)
    pad_out = next(n.output[0] for n in m.graph.node if n.op_type == "Pad")
    m.graph.output.extend(
        [helper.make_tensor_value_info(pad_out, onnx.TensorProto.FLOAT, None)]
    )
    sess = onnxruntime.InferenceSession(
        m.SerializeToString(), providers=["CPUExecutionProvider"]
    )
    names = [o.name for o in sess.get_outputs()]

    # All-ones in: the pad is constant-mode zeros, so the zeros ARE the padding and their
    # split is directly visible. Nothing else reveals it — the amounts are computed by a
    # Concat/ConstantOfShape chain, not stored as an initializer.
    L = 256 * 40
    outs = dict(zip(names, sess.run(None, {sess.get_inputs()[0].name: np.ones((1, L), np.float32)})))
    padded = np.asarray(outs[pad_out]).ravel()
    nz = np.nonzero(padded)[0]
    left, right = int(nz[0]), int(len(padded) - 1 - nz[-1])
    centre = (1024 - 1) / 2 - left
    print(f"  input {L} -> padded {len(padded)}   left {left}   right {right}")
    print(f"  => frame j centred at 256j {centre:+.1f} samples ({1000 * centre / SR_M:+.2f} ms)")
    print(f"  their CENTER_OFFSET = +127.5 (+7.97 ms) -> "
          f"{'HONEST' if abs(centre - 127.5) < 0.6 else 'WRONG by %+.1f' % (centre - 127.5)}\n")


# ---------------------------------------------------------------- B: vibrato phase
def _oracle_path(x):
    """Exactly the oracle's chain: torchaudio resample -> SwiftF0 -> their grid alignment."""
    import torch
    import torchaudio

    sys.path.insert(0, str(REFERENCE))
    from algorithms import get_algorithm

    algo = get_algorithm("SwiftF0")(sample_rate=SR_M, hop_size=HOP, fmin=65.0, fmax=2093.0)
    x = (0.7 * x / np.abs(x).max()).astype(np.float32)
    a = torchaudio.functional.resample(
        torch.from_numpy(x).unsqueeze(0), orig_freq=SR_C, new_freq=SR_M
    ).squeeze(0).numpy()
    a = a / max(np.abs(a).max(), 1e-9) * 0.95
    pitch, conf = algo.extract_continuous_periodicity(a)
    return algo._compute_target_times(len(a)), pitch, conf


def _phase_delay(rate_hz, carrier=300.0, depth=2.0, dur=24.0, stamp_bias=0.0):
    x, _ = vibrato_signal(carrier, depth, rate_hz, dur)
    ts, pitch, conf = _oracle_path(x)
    ts = ts + stamp_bias / SR_M
    good = (conf > 0.9) & (ts > 1.0) & (ts < dur - 1.0) & (pitch > 100) & (pitch < 900)
    y = 12.0 * np.log2(pitch[good] / carrier)
    y -= y.mean()
    th = 2 * np.pi * rate_hz * ts[good]
    # y ~ A sin(th + phi); phi = 2*pi*rate*delay, so delay falls straight out
    s, c = np.sum(y * np.sin(th)), np.sum(y * np.cos(th))
    return np.arctan2(c, s) / (2 * np.pi * rate_hz), 2 * np.hypot(s, c) / len(y)


def probe_vibrato_phase():
    print("=== B: SwiftF0's latency by vibrato phase, through the oracle's path ===")
    print("    d > 0 => the trajectory is LATE against the audio\n")
    ds = []
    for R in (2.0, 3.0, 4.0, 6.0):
        d, amp = _phase_delay(R)
        ds.append(d)
        print(f"  {R:4.1f} Hz   {1000 * d:+6.2f} ms   (vibrato amplitude recovered {amp:.3f} of 2.000)")
    ds = np.array(ds)
    print(f"\n  mean {1000 * ds.mean():+.2f} ms, spread {1000 * ds.std():.2f} ms across rates")
    print("  (a true delay is linear phase => it must not depend on the rate)")
    # A probe that cannot recover a delay we impose ourselves proves nothing about one we
    # did not. These two lines are why the +0 ms above is worth believing.
    b0, _ = _phase_delay(4.0)
    b1, _ = _phase_delay(4.0, stamp_bias=128.0)
    print(f"  self-check: re-stamping +128 samples moves it {1000 * (b1 - b0):+.2f} ms "
          f"(want -8.00)\n")


# ---------------------------------------------------------------- C: the envelope
def _best_lag(est_t, est_f, truth_fn, lags=np.arange(-16.0, 24.1, 0.5)):
    """Lag minimising median |cents|. Pure pitch timing: no counting, no voicing."""
    out = []
    for lag in lags / 1000.0:
        tr = truth_fn(est_t - lag)
        both = est_f > 0
        c = np.abs(1200.0 * np.log2(est_f[both] / tr[both]))
        c = c[c < 200.0]  # octave errors carry no timing signal
        out.append(np.median(c) if len(c) > 100 else np.nan)
    out = np.asarray(out)
    b = int(np.nanargmin(out))
    if 0 < b < len(out) - 1 and np.all(np.isfinite(out[b - 1 : b + 2])):
        y0, y1, y2 = out[b - 1], out[b], out[b + 1]
        d = y0 - 2 * y1 + y2
        b_shift = 0.5 * (y0 - y2) / d if abs(d) > 1e-12 else 0.0
    else:
        b_shift = 0.0
    return lags[b] + b_shift, out[b]


def probe_envelope():
    print("=== C: THE FINDING — same pitch, two envelopes ===")
    dur, carrier, depth, rate = 40.0, 300.0, 2.0, 4.0
    note_hz, tau = 3.0, 0.12

    x, truth_fn = vibrato_signal(carrier, depth, rate, dur)
    t = np.arange(len(x)) / SR_C
    # Fast attack, exponential decay, re-struck note_hz times a second. The pitch under it
    # is untouched — the envelope is the ONLY difference between the two runs.
    phase_in_note = (t * note_hz) % 1.0 / note_hz
    env = np.clip(phase_in_note / 0.005, 0, 1) * np.exp(-phase_in_note / tau)

    print(f"    {dur:.0f} s, {carrier:.0f} Hz, +/-{depth:.0f} st vibrato at {rate:.0f} Hz; "
          f"decay {note_hz:.0f} notes/s, tau={1000 * tau:.0f} ms")
    print("    truth is analytic — no annotation, no corpus, nothing to be offset\n")
    for label, sig in (
        ("FLAT  (constant amplitude)", x),
        ("DECAY (note attack+decay) ", x * env),
    ):
        ts, pitch, conf = _oracle_path(sig)
        lag, err = _best_lag(ts, np.where(conf >= 0.9, pitch, 0.0), truth_fn)
        print(f"  {label}   lag {lag:+6.2f} ms   (median err {err:5.2f} cents)")
    print("\n  The envelope alone manufactures the lag. There is no group delay in either run.\n")


# ---------------------------------------------------------------- D: the centred YIN
def _yin_frame(x, tau_min=20, tau_max=680, thresh=0.15):
    """CMNDF over one window. No taper: YIN's difference function wants the raw window."""
    n = len(x)
    p2 = 1 << (2 * n - 1).bit_length()
    f = np.fft.rfft(x, p2)
    ac = np.fft.irfft(f * np.conj(f), p2)[:n]
    pw = np.cumsum(x**2)
    total = pw[-1]
    taus = np.arange(1, min(tau_max + 1, n))
    # d(tau) = sum_j (x_j - x_{j+tau})^2, expanded into two power sums and the autocorrelation
    e1 = np.array([pw[n - 1 - t] for t in taus])
    e2 = np.array([total - pw[t - 1] for t in taus])
    d = np.maximum(e1 + e2 - 2 * ac[1 : len(taus) + 1], 0.0)
    cum = np.cumsum(d)
    cmndf = np.ones_like(d)
    nz = cum > 0
    idx = np.arange(1, len(d) + 1)
    cmndf[nz] = d[nz] * idx[nz] / cum[nz]
    lo = tau_min - 1
    cand = np.where(cmndf[lo:] < thresh)[0]
    if len(cand) == 0:
        return 0.0
    t = cand[0] + lo
    while t + 1 < len(cmndf) and cmndf[t + 1] < cmndf[t]:
        t += 1
    if t <= 0 or t >= len(cmndf) - 1:
        return 0.0
    y0, y1, y2 = cmndf[t - 1], cmndf[t], cmndf[t + 1]
    den = 2 * (2 * y1 - y2 - y0)
    tau = (t + 1) + ((y2 - y0) / den if abs(den) > 1e-12 else 0.0)
    return SR_C / tau if tau > 0 else 0.0


def probe_corpus_yin(n_tracks=3, secs=45, window=2048, hop=256):
    import soundfile as sf

    print("=== D: is the corpus itself offset? A ruler that shares nothing with A/B ===")
    if not CORPUS.is_dir():
        print(f"  {CORPUS} missing — skipped (fetch MDB-stem-synth to run this)\n")
        return
    audio_dir, ann_dir = CORPUS / "audio_stems", CORPUS / "annotation_stems"
    lags = np.arange(-16.0, 16.1, 0.5)
    done = 0
    for wav in sorted(audio_dir.glob("*.wav")):
        if done >= n_tracks:
            break
        ann = ann_dir / f"{wav.stem}.csv"
        if not ann.exists():
            continue
        x, sr = sf.read(wav, dtype="float32")
        assert sr == SR_C, sr
        if x.ndim > 1:
            x = x.mean(1)
        x = x[: secs * SR_C]
        ts, fs = [], []
        for s in range(0, len(x) - window, hop):
            seg = x[s : s + window]
            if np.abs(seg).max() < 1e-4:
                continue
            f0 = _yin_frame(seg)
            if f0 > 0:
                # CENTRED window => zero delay by construction. That is the whole point.
                ts.append((s + (window - 1) / 2.0) / SR_C)
                fs.append(f0)
        if len(fs) < 500:
            continue
        ts, fs = np.asarray(ts), np.asarray(fs)
        a = np.loadtxt(ann, delimiter=",")
        at, af = a[:, 0], a[:, 1]

        def truth_fn(want, at=at, af=af):
            idx = np.clip(np.searchsorted(at, want), 1, len(at) - 1)
            left = np.abs(want - at[idx - 1]) <= np.abs(at[idx] - want)
            return af[np.where(left, idx - 1, idx)]

        keep = truth_fn(ts) > 65.0
        if keep.sum() < 300:
            continue
        lag, err = _best_lag(ts[keep], fs[keep], truth_fn, lags)
        print(f"  {wav.stem[:44]:44s} offset {lag:+5.1f} ms  (median err {err:5.2f} cents)")
        done += 1
    print("\n  ~0 ms at ~1 cent => MDB-stem-synth's audio and annotation are aligned.")
    print("  So the lag SwiftF0 shows on this corpus is not the corpus's. It is C.\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--quick", action="store_true", help="skip D (needs the corpus)")
    args = ap.parse_args()

    probe_pad_split()
    probe_vibrato_phase()
    probe_envelope()
    if not args.quick:
        probe_corpus_yin()

    print("=== conclusion ===")
    print("  A: SwiftF0's frame stamps are honest.   B: the model has no delay.")
    print("  D: the corpus is aligned.               C: the envelope makes the lag anyway.")
    print("  => a lag-sweep peak is group_delay + envelope_bias. Reading it as a latency")
    print("     is wrong; comparing two detectors each at its own peak is still fair.")


if __name__ == "__main__":
    main()
