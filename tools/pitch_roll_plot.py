# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "matplotlib",
#     "numpy",
# ]
# ///
"""Render a pitch track as an intonation roll: pitch on a note grid, colour = how far off.

    SIEVE_DUMP_WAV=take.wav SIEVE_DUMP_FROM=26 SIEVE_DUMP_TO=36 SIEVE_DUMP_CSV=take.csv \\
      cargo test --release --lib sieve::tests::dump_track -- --ignored --nocapture
    uv run tools/pitch_roll_plot.py take.csv -o take.png --key G --temperament pythagorean

The CSV is `dsp::sieve::tests::dump_track`'s: `seconds,midi,score,level_dbfs`, MIDI against
A4 = 440, empty `midi` = no pitch. This script only draws; every decision about what the
pitch *is* was made by the detector.

# What the picture encodes

- **height** — pitch, on a semitone grid relabelled for `--a4` (a band tuned to A = 415
  plays its "A" on the A line, not a semitone low);
- **colour** — deviation from the nearest note *of the chosen temperament*: white inside
  `--in-tune` cents, then red when sharp and blue when flat, saturating at `--saturate`;
- **thickness** — the frame's RMS level, so a quiet harmonic ghost reads as the hairline
  it is and a sustained note as a band.

Pythagorean is a chain of pure fifths (701.955 ¢) built around `--key`'s tonic, which is
itself held at its equal-tempered place: pitch class `i` semitones above the tonic, reached
by `n` fifths with `7n ≡ i (mod 12)` and `n ∈ −5..6`, sits `n · 1.955 ¢` off equal temperament.
"""

import argparse
import csv
import math

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402

NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
FLATS = {"Db": 1, "Eb": 3, "Gb": 6, "Ab": 8, "Bb": 10}
PYTHAGOREAN_COMMA_PER_FIFTH = 1200 * math.log2(1.5) - 700  # 1.955 cents


def pitch_class(name: str) -> int:
    if name in FLATS:
        return FLATS[name]
    return NAMES.index(name)


def temperament_offsets(temperament: str, tonic: int) -> list[float]:
    """Cents off equal temperament for each pitch class 0..11."""
    if temperament == "equal":
        return [0.0] * 12
    offsets = [0.0] * 12
    for fifths in range(-5, 7):
        pc = (tonic + 7 * fifths) % 12
        offsets[pc] = fifths * PYTHAGOREAN_COMMA_PER_FIFTH
    return offsets


def read_track(path: str):
    seconds, midi, level = [], [], []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            if not row["midi"]:
                continue
            seconds.append(float(row["seconds"]))
            midi.append(float(row["midi"]))
            level.append(float(row["level_dbfs"]))
    return np.array(seconds), np.array(midi), np.array(level)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("csv")
    ap.add_argument("-o", "--out", required=True)
    ap.add_argument("--a4", type=float, default=440.0, help="display tuning, Hz")
    ap.add_argument("--temperament", choices=["equal", "pythagorean"], default="equal")
    ap.add_argument("--key", default="C", help="tonic for pythagorean, e.g. G, Eb")
    ap.add_argument("--low", type=int, default=53, help="lowest MIDI row (F3)")
    ap.add_argument("--high", type=int, default=86, help="highest MIDI row (D6)")
    ap.add_argument("--in-tune", type=float, default=5.0, help="cents drawn white")
    ap.add_argument("--saturate", type=float, default=30.0, help="cents at full colour")
    ap.add_argument("--title", default="")
    args = ap.parse_args()

    seconds, midi, level = read_track(args.csv)
    # Re-express against the display tuning: MIDI 69 is then the band's A, whatever its Hz.
    midi = midi - 12 * math.log2(args.a4 / 440.0)
    offsets = temperament_offsets(args.temperament, pitch_class(args.key))
    nearest = np.round(midi).astype(int)
    target = nearest + np.array([offsets[n % 12] for n in nearest]) / 100.0
    cents = (midi - target) * 100.0

    # white → red (sharp) / blue (flat), linear in |cents| past the in-tune band.
    strength = np.clip((np.abs(cents) - args.in_tune) / (args.saturate - args.in_tune), 0, 1)
    colours = np.ones((len(cents), 3))
    sharp, flat = cents > 0, cents < 0
    colours[sharp, 1] -= strength[sharp]
    colours[sharp, 2] -= strength[sharp]
    colours[flat, 0] -= strength[flat]
    colours[flat, 1] -= strength[flat]

    # Thickness from level: −60 dBFS and below is a hairline, 0 dBFS a third of a semitone.
    half_height = 0.02 + 0.15 * np.clip((level + 60.0) / 60.0, 0, 1)

    fig, ax = plt.subplots(figsize=(10, 6), dpi=100)
    fig.patch.set_facecolor("#d4d0c8")
    ax.set_facecolor("black")
    for row in range(args.low, args.high + 1):
        natural = NAMES[row % 12].find("#") < 0
        ax.axhline(row, color="#9a9a9a" if natural else "#3a3a3a", lw=0.6, zorder=1)
    ax.vlines(seconds, midi - half_height, midi + half_height, colors=colours, lw=1.2, zorder=2)

    ax.set_ylim(args.low - 0.5, args.high + 0.5)
    ax.set_xlim(seconds.min() if len(seconds) else 0, seconds.max() if len(seconds) else 1)
    ax.set_yticks([r for r in range(args.low, args.high + 1) if "#" not in NAMES[r % 12]])
    ax.set_yticklabels([f"{NAMES[r % 12]}{r // 12 - 1}" for r in ax.get_yticks()], fontsize=7)
    ax.xaxis.tick_top()
    ax.grid(axis="x", color="#555555", lw=0.5)
    tuning = "equal" if args.temperament == "equal" else f"pythagorean on {args.key}"
    ax.set_title(f"{args.title}  ·  {tuning}, A = {args.a4:g} Hz", fontsize=9, pad=18)
    fig.tight_layout()
    fig.savefig(args.out)


if __name__ == "__main__":
    main()
