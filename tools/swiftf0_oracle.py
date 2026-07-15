# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = [
#     "swift-f0==0.1.2",
#     "numpy",
#     "pandas",
#     "scipy",
#     "torch",
#     "torchaudio",
#     "torchcodec",
#     "soundfile",
#     "librosa",
#     "numba>=0.61",
#     "tqdm",
# ]
# ///
#
# `numba` is not imported here — it is pinned because without a floor the resolver walks
# librosa's dependency down to numba 0.53 / llvmlite 0.36, which predate Python 3.10 and
# fail to build. It only ever resolved on this machine by reusing a cached environment;
# on a cold cache the script did not start.
#
# `torchcodec` is here because torchaudio 2.11 removed its built-in loaders and routes
# `torchaudio.load` through it; the reference's `requirements.txt` predates that and
# pins torchaudio 2.7.1, which still decoded on its own. We do not pin back to their
# versions: it costs a multi-GB re-download of CUDA wheels to change what decodes a
# lossless wav and resamples it to 16 kHz. `run.json` records what actually ran, so a
# number can always be traced to an environment.
#
# torch comes from PyPI with its CUDA wheels even though this only ever runs on CPU:
# pointing it at download.pytorch.org/whl/cpu drives the resolver into a corner where it
# backtracks to numba 0.53 / llvmlite 0.36, which build on no Python this script accepts.
# A couple of GB of unused disk is the cheaper bug.
"""Run SwiftF0 over MDB-stem-synth **through the reference harness**, and dump its pitch
trajectory as CSV for `dsp::pitch_bench` to score.

Usage (needs nothing pre-installed but `uv`):

    uv run tools/swiftf0_oracle.py                 # whole corpus
    uv run tools/swiftf0_oracle.py --limit 3       # smoke run
    uv run tools/swiftf0_oracle.py --filter violin

# Why this exists

Our RPA number has never been checked against anything outside this repo. `testdata/`'s
violin takes and MDB-stem-synth both answer "is the detector right"; neither answers "is
our *scoring* right". A benchmark that is wrong in its own favour is worse than none.

So: take an estimator with published numbers (SwiftF0 — MIT, 398 KB ONNX, ranked #1 in
`lars76/pitch-benchmark`), run it, and score its trajectory with our harness. If our
scorer reproduces the reference's verdict on the *same* trajectory, our arithmetic is
sound and our own numbers mean something. That is the only claim this script supports.

# Why it imports the reference instead of reimplementing it

Everything that decides a number — how the corpus is read, how the annotation is
resampled onto the frame grid, how audio is normalised, how the model is driven, how the
metric is computed — is imported from `lars76/pitch-benchmark` at a pinned commit. A
reimplementation would be one more thing that could be wrong, and the whole point is to
be checked *by* code we did not write.

# The one deliberate deviation: no noise

`pitch_benchmark.py`'s `__main__` unconditionally wraps the corpus in
`CHiMeNoiseDataset` — CHiME-Home plus Gaussian noise, SNR 10–30 dB, random voice gain
−6…+6 dB. There is no clean path through their runner. Their published cell
(`SwiftF0 × MDBStemSynth = 92.0 %`) is therefore a *noisy*-audio number.

We score clean audio, so this script builds the base dataset and skips the wrapper. The
consequence is stated plainly rather than papered over: **the numbers printed here are
not comparable to their published table** and are not meant to be. What travels between
the two harnesses is the *trajectory*, and the check is that two independent scorers
agree on it.

# Three ways their metric differs from ours — read before comparing any number

1. **Their "RPA" is not RPA.** `run_single_evaluation` builds the denominator as
   `pred_voicing & true_voicing`, so a frame the annotation calls voiced and the
   algorithm calls silent is *dropped from the denominator* instead of counted as a
   miss. That is pitch accuracy *conditional on the detector speaking*. mir_eval — and
   `score_frame` in `dsp::pitch_bench`, which counts such a frame as `silent_miss` — divide
   by every voiced frame of the annotation. Their figure is therefore optimistic against
   ours by construction, not by quality; their harmonic mean re-applies the penalty
   through a separate `voicing_recall` term.
2. **Their MDB mask is 65–2093 Hz**, from `PitchDatasetMDBStemSynth.fmin/fmax` — not
   SwiftF0's own 46.875–2093.75 Hz range. With `clip_pitch=False` (their default),
   annotation outside that band keeps its value but has its periodicity zeroed, i.e. it
   silently stops being voiced.
3. **They resample the annotation by linear interpolation in Hz** onto
   `len(audio) // hop_size` frames (`align_corners=True`); our `Annotation::at` takes the
   nearest neighbour and interpolates nothing.

`run.json` records everything needed to account for all three when the two numbers are
put side by side.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
from tqdm import tqdm

# The reference harness, pinned. A moving `main` would make every number here
# irreproducible the moment upstream edits a metric — and the metric is the thing we are
# borrowing, so it is the last thing that may drift silently.
REFERENCE_REPO = "https://github.com/lars76/pitch-benchmark.git"
REFERENCE_COMMIT = "87982db236ac7fd05bf2cda4ed39331bafecb7fc"

# Their runner's defaults (`pitch_benchmark.py --sample-rate 16000 --hop-size 256`).
# 256/16000 s = 16 ms, which is exactly `BANK_HOP_SECONDS` in `dsp::pitch_bench` — so the
# oracle's frames land on the same grid as the bank's and a per-frame diff needs no
# resynchronisation. Coincidence, but a load-bearing one: do not "tidy" either constant.
SAMPLE_RATE = 16000
HOP_SIZE = 256

# Their threshold sweep, verbatim (`np.linspace(0.0, 1.0, 11)`), and their selection rule:
# whichever threshold maximises the combined harmonic-mean score over the *whole* corpus.
# Kept because the published numbers are produced this way; it means the result is an
# oracle-tuned ceiling, not what a fixed threshold would give in production.
THRESHOLDS = np.linspace(0.0, 1.0, 11)

PROJECT_ROOT = Path(__file__).resolve().parent.parent


def ensure_reference(repo_dir: Path) -> None:
    """Clone the reference harness at `REFERENCE_COMMIT` if it is not already there.

    Lives under `datasets/` because that tree is git-ignored: this is third-party code
    fetched on demand, exactly like the corpus it scores, and vendoring it would put a
    second copy of someone else's benchmark in our history.
    """
    if (repo_dir / "pitch_benchmark.py").is_file():
        head = subprocess.run(
            ["git", "-C", str(repo_dir), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        # A checkout at the wrong commit is worse than no checkout: it would produce
        # numbers attributed to a commit that did not make them.
        assert head == REFERENCE_COMMIT, (
            f"{repo_dir} is at {head}, expected {REFERENCE_COMMIT}. "
            f"Delete it and re-run to re-clone."
        )
        return

    print(f"fetching reference harness -> {repo_dir}")
    repo_dir.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "clone", REFERENCE_REPO, str(repo_dir)], check=True)
    subprocess.run(["git", "-C", str(repo_dir), "checkout", REFERENCE_COMMIT], check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--corpus",
        type=Path,
        default=PROJECT_ROOT / "datasets" / "MDB-stem-synth",
        help="MDB-stem-synth root (the dir holding audio_stems/ and annotation_stems/)",
    )
    parser.add_argument(
        "--reference",
        type=Path,
        default=PROJECT_ROOT / "datasets" / "reference" / "pitch-benchmark",
        help="where to clone lars76/pitch-benchmark",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=PROJECT_ROOT / "datasets" / "oracle_swiftf0",
        help="output dir for <track>.csv + run.json",
    )
    parser.add_argument("--filter", default="", help="substring of the track id")
    parser.add_argument("--limit", type=int, default=0, help="max tracks (0 = all)")
    args = parser.parse_args()

    ensure_reference(args.reference)
    # Their modules import each other by top-level name (`from datasets import ...`), so
    # the repo root has to be on the path as if we were running from inside it.
    sys.path.insert(0, str(args.reference))

    # Imported, never copied — see the module docstring. `algorithms/__init__` is lazy, so
    # asking for SwiftF0 does not drag in tensorflow/penn/crepe.
    from algorithms import get_algorithm
    from datasets import get_pitch_dataset
    from pitch_benchmark import (
        calculate_combined_score,
        calculate_processed_metrics,
        evaluate_pitch_accuracy,
        evaluate_voicing_detection,
    )

    dataset = get_pitch_dataset("MDBStemSynth")(
        root_dir=str(args.corpus),
        sample_rate=SAMPLE_RATE,
        hop_size=HOP_SIZE,
        # Their default is `use_cache=True`, which retains every decoded waveform for the
        # lifetime of the run — 15.5 hours of 16 kHz float32, about 3.6 GB, to serve a loop
        # that reads each track exactly once. Measured at 5.6 GB RSS a quarter of the way
        # in, heading for ~20 GB against 16 GB free. Caching cannot change a value, so
        # turning it off is a deviation in resource use only, not in what is computed.
        use_cache=False,
    )
    # NOTE: `CHiMeNoiseDataset` is deliberately *not* wrapped around `dataset` here. This
    # is the single deviation from their runner; see the module docstring.

    algorithm = get_algorithm("SwiftF0")(
        sample_rate=dataset.sample_rate,
        hop_size=dataset.hop_size,
        fmin=dataset.fmin,
        fmax=dataset.fmax,
    )

    selected = list(range(len(dataset)))
    if args.filter:
        selected = [
            i
            for i in selected
            if args.filter.lower() in dataset.wav_f0_pairs[i][0].stem.lower()
        ]
    if args.limit:
        selected = selected[: args.limit]
    assert selected, f"no tracks matched filter {args.filter!r}"

    args.out.mkdir(parents=True, exist_ok=True)
    print(f"=== SwiftF0 via lars76/pitch-benchmark@{REFERENCE_COMMIT[:8]} ===")
    print(f"    corpus : {args.corpus}  ({len(selected)} tracks)")
    print(f"    audio  : CLEAN (their CHiMe noise wrapper skipped)")
    print(f"    mask   : {dataset.fmin}-{dataset.fmax} Hz (theirs, for MDB)")

    # Per-track continuous output, held so the threshold sweep and the CSV dump can share
    # one pass of inference. `extract_continuous_periodicity` is what their
    # `_extract_continuous_multiple_thresholds` calls once before looping over thresholds;
    # thresholding it with `confidence >= t` is that loop's body, minus the note
    # segmentation their metrics do not read. Same arithmetic, one model run.
    tracks: list[dict] = []
    skipped = 0

    for idx in tqdm(selected, desc="SwiftF0", unit=" track"):
        sample = dataset[idx]
        audio = sample["audio"].numpy()
        true_pitch = sample["pitch"].numpy()
        true_voicing = sample["periodicity"].numpy()

        # Their runner's skip rule: a track the annotation calls silent throughout has no
        # denominator to contribute and is dropped rather than scored as zero.
        if not true_voicing.any():
            skipped += 1
            continue

        pitch, confidence = algorithm.extract_continuous_periodicity(audio)
        # Their grid, via their own helper: frame i is at i * hop / sr. Reaching for a
        # private method is intentional — recomputing the grid ourselves is exactly the
        # kind of "obviously equivalent" copy that silently drifts by one frame.
        times = algorithm._compute_target_times(len(audio))

        tracks.append(
            {
                "id": sample["wav_path"].stem,
                "times": times,
                "pitch": pitch,
                "confidence": confidence,
                "true_pitch": true_pitch,
                "true_voicing": true_voicing,
            }
        )

    if skipped:
        print(f"    skipped {skipped} fully-unvoiced tracks (their rule)")

    # The sweep, aggregated exactly as `run_single_evaluation` does it: concatenate every
    # track into one flat array *per threshold*, then score once. Not a mean of per-track
    # scores — a long track therefore weighs more than a short one, which is their choice
    # and has to be reproduced to reproduce their number.
    best = None
    for threshold in THRESHOLDS:
        pred_voicing = np.concatenate([(t["confidence"] >= threshold) for t in tracks])
        true_voicing = np.concatenate([t["true_voicing"] for t in tracks])
        pred_pitch = np.concatenate([t["pitch"] for t in tracks])
        true_pitch = np.concatenate([t["true_pitch"] for t in tracks])

        voicing_metrics = evaluate_voicing_detection(pred_voicing, true_voicing)
        pitch_metrics = evaluate_pitch_accuracy(
            pred_pitch, true_pitch, pred_voicing & true_voicing
        )
        score = calculate_combined_score(voicing_metrics, pitch_metrics)
        print(
            f"    threshold {threshold:.1f}: HM {100 * score:5.2f}%  "
            f"their-RPA {100 * pitch_metrics['rpa']:5.2f}%  "
            f"({pitch_metrics['valid_frames']} scored frames)"
        )
        if not np.isnan(score) and (best is None or score > best["score"]):
            best = {
                "score": score,
                "threshold": float(threshold),
                "voicing": voicing_metrics,
                "pitch": pitch_metrics,
                "processed": calculate_processed_metrics(voicing_metrics, pitch_metrics),
            }

    assert best is not None, "every threshold scored NaN — the run is broken, not close"

    def write_csv(path: Path, times: np.ndarray, hz: np.ndarray, voiced: np.ndarray) -> None:
        """`time,frequency`, 0 Hz = unvoiced.

        Byte-identical in convention to the corpus's own annotation CSVs, which is what
        lets `dsp::pitch_bench::load_annotation` read these files with no new parser.
        """
        lines = [f"{t:.6f},{f if v else 0.0:.6f}" for t, f, v in zip(times, hz, voiced)]
        path.write_text("\n".join(lines) + "\n")

    # Dump at the winning threshold: the CSVs must correspond to the numbers printed
    # above, or the cross-check would compare our scorer against a trajectory the
    # reference never scored.
    for track in tracks:
        write_csv(
            args.out / f"{track['id']}.csv",
            track["times"],
            track["pitch"],
            track["confidence"] >= best["threshold"],
        )

    # The reference's *annotation*, on the reference's grid — the truth their metric
    # actually scored against, which is not the corpus's annotation.
    #
    # `PitchDataset.process_sample` resamples the 2.9 ms annotation onto the frame grid
    # with `F.interpolate(..., mode="linear", align_corners=True)`, and align_corners maps
    # annotation index i onto frame i*(L-1)/(N-1). For a 171 s track that is frame j
    # reading the truth from 0.0160013·j seconds while the harness calls it 0.016·j — a
    # drift that reaches ~14 ms, nearly a whole frame, by the end of a long track. On
    # ~5.8 % of voiced frames their truth ends up more than 50 cents from the corpus's.
    #
    # So: dumping it is not a courtesy, it is what makes the harness comparison decidable.
    # Scored against the corpus annotation, our RPA and theirs differ because the *truth*
    # differs and no amount of staring at the scorers explains it. Scored against this
    # file, the annotation is held fixed and any remaining gap is arithmetic — which is the
    # only thing the cross-check is entitled to conclude about.
    #
    # Their band mask is already baked in: `_validate_pitch` zeroes periodicity outside
    # 65-2093 Hz, so those frames are written as unvoiced here.
    truth_dir = args.out / "truth"
    truth_dir.mkdir(parents=True, exist_ok=True)
    for track in tracks:
        write_csv(
            truth_dir / f"{track['id']}.csv",
            track["times"],
            track["true_pitch"],
            track["true_voicing"],
        )

    manifest = {
        "reference_repo": REFERENCE_REPO,
        "reference_commit": REFERENCE_COMMIT,
        "audio": "clean (CHiMeNoiseDataset skipped — see script docstring)",
        "sample_rate": SAMPLE_RATE,
        "hop_size": HOP_SIZE,
        "hop_seconds": HOP_SIZE / SAMPLE_RATE,
        "mask_hz": [dataset.fmin, dataset.fmax],
        "thresholds_swept": [float(t) for t in THRESHOLDS],
        "optimal_threshold": best["threshold"],
        "tracks": [t["id"] for t in tracks],
        "skipped_unvoiced_tracks": skipped,
        # `truth/<track>.csv` holds the reference's own resampled annotation. It is the
        # denominator's definition of truth, and it drifts from the corpus's — see the
        # comment at its dump site.
        "truth_subdir": "truth",
        # Their verdict, by their code, on this (clean) audio. The comparison target for
        # `dsp::pitch_bench`'s external-trajectory mode.
        "reference_metrics": {
            "note": (
                "'rpa' here is THEIR definition: denominator is pred_voicing & "
                "true_voicing, so frames where the detector went silent on voiced truth "
                "are excluded rather than counted as misses. Not mir_eval RPA."
            ),
            "combined_score_harmonic_mean": float(best["score"]),
            "voicing": {k: float(v) for k, v in best["voicing"].items()},
            "pitch": {
                k: (None if v is None or (isinstance(v, float) and np.isnan(v)) else float(v))
                for k, v in best["pitch"].items()
            },
        },
    }
    (args.out / "run.json").write_text(json.dumps(manifest, indent=2) + "\n")

    print(f"\n=== best threshold {best['threshold']:.1f} (their selection rule) ===")
    print(f"  HM combined score  : {100 * best['score']:.2f}%")
    print(f"  their-RPA          : {100 * best['pitch']['rpa']:.2f}%   <- NOT mir_eval RPA")
    print(f"  their-RCA          : {100 * best['pitch']['rca']:.2f}%")
    print(f"  voicing recall     : {100 * best['voicing']['recall']:.2f}%")
    print(f"  voicing precision  : {100 * best['voicing']['precision']:.2f}%")
    print(f"  cents error (mean) : {best['pitch']['cents_error']:.1f}")
    print(f"  scored frames      : {best['pitch']['valid_frames']}")
    print(f"\n  wrote {len(tracks)} trajectories + run.json -> {args.out}")
    print(
        "\n  Their published MDBStemSynth cell is 92.0% HM on CHiME-noised audio; this run\n"
        "  is clean, so the two are not comparable. What crosses between harnesses is the\n"
        "  trajectory, not the number."
    )


if __name__ == "__main__":
    main()
