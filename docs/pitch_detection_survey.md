# Pitch-detection algorithm survey — for the violin trainer

Notes for "какая нота играет сейчас" + latency work. Companion to
[`violin_trainer_plan.md`](violin_trainer_plan.md). What this app already has, what
the literature offers, and the concrete next steps.

> **For how the mechanism is actually wired today, read
> [`note_detection.md`](note_detection.md)** — this file is the *survey* (what
> algorithms exist and why these were chosen); that one is the *description* (what
> runs, where, at what cadence, with the measured numbers).

## What the app already does

> **Status note (Phase 1.7).** The "wins come from marrying them" line below turned
> out to be exactly right, and the numbers here were confirmed by measurement — see
> `violin_trainer_plan.md` Phase 1.7. The marriage only works with the **bank
> leading** and YIN pinning the octave (`audio::dsp::melody`). The other
> arrangement — fusing the bank into the pYIN HMM as a weighted candidate — was
> built (Phase 1.5) and **removed** (Phase 1.12): off an onset it contributed exactly
> zero, because YIN's own candidate scores `p = 1.000` against the bank's capped
> `≤ 0.5`; on an onset it made pYIN echo the bank's octave, so the two "independent"
> sources were confirming each other rather than voting.

- **YIN / pYIN** (`audio::dsp::pyin`) — the octave anchor, and the pitch source for
  the tuner and fretboard. Time-domain autocorrelation-difference. Accurate & robust
  for a monophonic line, gives cents. **Slow by construction**: window ≈ 6144 samples
  ≈ **128 ms** + a 40 ms re-analysis cadence. Measured: note-change latency equals
  the window length *exactly*, because the HMM will not leave a note while the window
  still holds any trace of it. This was the source of the staff's note-commit lag —
  the melody line no longer reads it.
- **Resonator bank** (`audio::dsp::resonator`, `OnePoleBank`) — the *fast* channel.
  Per-sample IIR leaky resonators, published ~**16 ms (60 Hz)**. Includes a Δφ
  **instantaneous-frequency reassignment** (super-resolution + coherence-gate that
  suppresses the negative-frequency image / noise). This is already a "paper-grade"
  technique (the reassignment method). Fed to the staff waterfall (Phase 1.2).
  **Measured note-change latency: 8–29 ms** (`resonator::tests::bank_latency_probe`)
  — inside the perceptual threshold below. This is the melody line's pitch source
  (Phase 1.7); its one weakness is the octave, which is what YIN is borrowed for.

So we have a slow-but-clean estimate (YIN) and a fast energy/pitch view (resonators).
The wins come from marrying them.

## Algorithms surveyed

### Classical, time-domain (cheap, pure-Rust friendly)
- **YIN** — de Cheveigné & Kawahara, 2002. What we use.
  <https://www.researchgate.net/publication/11367890_YIN_A_fundamental_frequency_estimator_for_speech_and_music>
- **pYIN** — Mauch & Dixon, 2014. Probabilistic YIN: emits *multiple* pitch
  candidates with probabilities, then an **HMM** picks the smoothest path.
  **Directly kills octave errors** and jitter (our Phase 1.1/2 follow-up) with
  little extra latency. **Top recommended upgrade.** ✅ **IMPLEMENTED** in
  `audio::dsp::pyin` — Beta-threshold candidates + online-Viterbi HMM; replaced the
  plain-YIN pipeline path. See violin plan Phase 1.4.
- **SWIPE′** (Camacho, 2008), **adaptive comb filters** — fast, low-latency.
  Comb/adaptive-comb: <https://ieeexplore.ieee.org/document/1035730/>

### Time-frequency
- **Pseudo Wigner-Ville distribution (PWVD)** — Liu, Wu, Black, Anumanchipalli,
  2022, "A Fast and Accurate Pitch Estimation Algorithm Based on the PWVD".
  <https://arxiv.org/abs/2210.15272> — see the dedicated note below.
- **Minimum-latency / asymmetric analysis windows** — Shimauchi et al., 2016.
  Cuts the *group delay* of the analysis, i.e. attacks latency at the window level
  (a lever for the YIN path). <https://arxiv.org/pdf/1606.09047>

### Data-driven — surveyed properly 2026-07-15

> The one-line summary: **the trend is not bigger, it is smaller** — 22M params (CREPE,
> 2018) → 28.9k (PESTO, 2023) → **647** (SWIPE-tiny, 2025) — and the smallest of them
> wins by *not* learning a frontend from scratch. Which is the same bet `dsp::swipe`
> already made.

**There are no transformers here.** For monophonic f0 they did not win: the SOTA is small
CNNs. Transformers own the *neighbouring* task — polyphonic transcription
([hFT-Transformer](https://arxiv.org/abs/2307.04305), Sony AI, SOTA on MAESTRO;
[MT3](https://github.com/magenta/mt3), Magenta) — which is offline, heavy, and about
turning a piano into MIDI, not about "what note is sounding now". Not our problem.

| Model | Params | Frontend | Licence | Verdict here |
|---|---|---|---|---|
| [CREPE](https://arxiv.org/abs/1802.06182) 2018 | 22M (tiny 487k) | raw waveform, 64 ms | MIT | **superseded** — SwiftF0 beats it by 12+ points |
| [PESTO](https://arxiv.org/abs/2508.01488) ISMIR'23 / TISMIR'25 | 28.9k | CQT | LGPL-3.0 | self-supervised SOTA; ONNX export; but see SWIPE below |
| [RMVPE](https://arxiv.org/abs/2306.15412) 2023 | heavy | mel | — | vocals *inside polyphony* — not our case |
| [SwiftF0](https://arxiv.org/abs/2508.18440) 2025 | 96k | STFT 1024@16k | **MIT** | `model.onnx` = **398 KB**; demo already runs on wasm+ONNX; 46.9–2093.75 Hz (violin E7 is outside) |
| [FCPE](https://arxiv.org/html/2509.15140v1) 2025 | — | mel | — | fastest: 2.6× PESTO, 77× CREPE; 96.79 % RPA on MIR-1K |
| [Basic Pitch](https://github.com/spotify/basic-pitch) | ~17k | CQT + harmonic stacking | Apache | polyphonic — a different question (chords) |

**The one that matters:
[Improving Neural Pitch Estimation with SWIPE Kernels](https://arxiv.org/abs/2507.11233)**
(Marttila & Reiss, ISMIR 2025). They replace the neural frontend with **SWIPE kernel
scores** — i.e. with the thing `dsp::swipe` already computes — and find:

1. **SWIPE-tiny: 647 parameters** beats PESTO's 28.9k (96.6 % vs 94.6 % RPA, MIR-1K).
   The architecture is a *single* Toeplitz layer: "a convolutional layer with a single
   filter of size 647" plus softmax normalisation. **One convolution over the salience
   curve** — in Rust that is a loop, `[f32; 647]` = 2.6 KB in the binary, and **no ONNX,
   no tract, no new dependency**.
2. **Plain SWIPE outperforms every self-supervised neural pitch estimator published**
   (their §5.2, emphasis theirs). Our chosen path was already the right one.
3. **Their Table 1** — the most useful table in the paper, and the reason
   [`pitch_benchmark.md`](pitch_benchmark.md) now exists: SWIPE is reported at 86.6 % in
   the literature and measures **96.2 %** in a careful implementation. Ten points between
   implementations of one algorithm.

**But read the noise columns before reaching for it.** On clean audio the Toeplitz layer
buys **+0.4 points** over plain SWIPE (96.6 vs 96.2). Its actual purchase is robustness:
at −10 dB SNR it is 88.5 % vs SWIPE's 75.2 %, +13 points. A quiet room is the regime where
the network is worth almost nothing.

Two further practicalities, both measured rather than guessed:

- **Code and weights are advertised but gone.** The paper's footnote points at
  `github.com/dsuedholt/neural-pitch-swipe`; the account does not exist (404) and the
  repo is not among the author's 32 public ones. Training it is on us.
- **Grids do not match.** They use 3 bins/semitone over 27.5–8055 Hz (295 candidate bins),
  sampling the spectrum at 1024 mel-spaced frequencies. We run **8 bins/semitone**
  (`SPIRAL_BINS_PER_SEMITONE`, 12.5 cents — 2.7× finer) over MIDI 24–108, off the
  resonator bank rather than an FFT. Weights would not transfer even if they existed; the
  647-tap kernel is tied to its input resolution.
- **It would not make us faster.** Their Table 4: 93 ms window → 96.2 % RPA, but 46 ms →
  85.0 %. So it is still a *slow-plane* estimator — a replacement for pYIN as the octave
  anchor (128 → ~93 ms), never for the bank. Pleasant bonus: window size is an
  inference-time knob, **no retraining**.

Benchmarks / comparisons: [pitch-benchmark (lars76)](https://github.com/lars76/pitch-benchmark),
[Comparing PDAs vs data-driven (arXiv 2206.14357)](https://arxiv.org/pdf/2206.14357),
[RT F0 via spectrogram+CNN (arXiv 2504.06165)](https://arxiv.org/pdf/2504.06165).

### Data-driven, second pass — causal/real-time angle (2026-07-15)

Second sweep, this time asking the question the first pass didn't: **does any network
beat the bank's 8–29 ms?** Answer: **no, and none can** — every estimator surveyed needs
~4–8 periods of signal in its analysis window, so latency is set by physics, not by the
model class. What the networks buy is noise robustness on the *slow* plane, which the
first pass already established. The rest of the sweep, model by model:

- **[penn / FCNF0++](https://github.com/interactiveaudiolab/penn)** (Morrison et al. 2023,
  [arXiv 2301.12258](https://arxiv.org/pdf/2301.12258)) — CREPE modernised: 1440 bins
  (5 cents), 10 ms hop, entropy-based periodicity. The training-recipe paper (its `++`
  practices are what CREPE++/DeepF0++ mean elsewhere). On the lars76 benchmark it scores
  **84.8 %** — below Praat (84.7 % but faster) and well below SwiftF0. Window still
  ~128 ms of 8 kHz audio. Superseded; skip.
- **[HarmoF0](https://arxiv.org/pdf/2205.01019)** (2022) — log-scale dilated convolutions,
  per-frame. Nice trick (dilation = harmonic spacing), but non-causal, heavier than PESTO,
  and PESTO/SwiftF0 pass it. Skip.
- **[SPICE](https://github.com/lars76/pitch-benchmark)** (Google 2019, self-supervised) —
  benchmarked by lars76, lands mid-pack, superseded by PESTO on every axis. Skip.
- **[TAPE](https://github.com/MTG/tape)** ([ICASSP 2023](https://ieeexplore.ieee.org/document/10096762/),
  MTG/AudioLabs) — **violin-specific**, timbre-aware: estimates *the violin's* pitch inside
  a violin–piano duet without source separation (two conv streams + a transformer, E3–E8,
  480 bins). Solves a problem we don't have (accompaniment cross-talk), with hardware we
  don't want to spend. Bookmark for a hypothetical "play along with accompaniment" mode;
  irrelevant for solo latency.
- **[PESTO's real-time numbers](https://transactions.ismir.net/articles/10.5334/tismir.251)**
  (TISMIR 2025), now with specifics: max kernel = 8192 samples @ 48 kHz = **171 ms window,
  ~85 ms inherent lag**; streamable cached-convolution VQT; <10 ms compute. So "real-time"
  means *throughput*, not group delay — slow-plane only, and our pYIN at 128 ms already
  sits in the same band with a better octave anchor story.
- **[lars76/pitch-benchmark](https://github.com/lars76/pitch-benchmark)** leaderboard
  (13 algorithms, 8 datasets): SwiftF0 **90.2 %**, RMVPE 87.2 % (best on vocals), CREPE
  85.3 %, PENN 84.8 %, Praat 84.7 % (fastest classical, 2.8 ms/s audio). Confirms the
  first pass: SwiftF0 is the only off-the-shelf net worth anything here — and its use for
  us would be as an **independent oracle for `pitch_bench`** (398 KB MIT ONNX, runs
  anywhere), a non-circular cross-check of our RPA harness, not a runtime component.

**The find of the sweep —
[RT-SWIPE](https://www.audiolabs-erlangen.de/content/05_fau/professor/00_mueller/03_publications/2025_MeierSSMB_RealTimeSWIPE_CMMR_ePrint.pdf)**
(Meier, Strahl, Schwär, Müller, Balke — CMMR 2025, AudioLabs Erlangen). SWIPE made causal
by **right-aligning** the per-candidate analysis windows against the newest sample
(instead of centring them, which costs a constant N_max/2 delay). Delay becomes
pitch-dependent: δ = N_i/2 = **4 periods of the pitch** — 400 Hz → ~10 ms, 200 Hz →
~20 ms, 100 Hz → ~40 ms, 50 Hz → ~80 ms. Under strict frame-aligned RPA it "loses"
(0.931 vs offline SWIPE's 0.960 on ChoraleBricks), but with a **time-tolerant RPA**
(accept estimates arriving within τ; at τ = 46.4 ms) it matches the offline baseline —
i.e. the errors were *delay*, not *scoring*. Three consequences for us:

1. **Independent validation of `pitch_bench`'s lag sweep.** Their time-tolerant RPA is
   the same diagnosis our lag sweep made (half our "errors" were timing) — published,
   citable, same numbers band (their τ ≈ 23–46 ms vs our peak at 24 ms).
2. **The bank already beats RT-SWIPE's delay curve.** Their causal SWIPE pays 4 periods;
   the reassigned bank publishes at 8–29 ms across the whole range — at 100 Hz that is
   29 ms vs their 40 ms, and the gap widens as pitch drops. Our fast plane is not just
   competitive with the 2025 causal-classical frontier, it is ahead of it.
3. Same group also published **[dYIN / dSWIPE](https://doi.org/10.1109/TASLPRO.2025.3581119)**
   (IEEE TASLPRO 2025) — *differentiable* variants with trainable spectral templates,
   explicitly aimed at per-instrument kernels. If we ever train anything, the menu is now
   two entries: SWIPE-tiny's 647-tap Toeplitz layer (noise robustness) or dSWIPE-style
   violin-tuned kernels (cross-talk robustness). Both keep the SWIPE frontend we have.

Verdict of the second pass: **the "совсем идеальное" does not exist** — the 2025/26
literature converged on exactly our architecture (causal SWIPE-family scorer + a
delay-aware metric), and where it differs, we are ahead (IIR bank vs windowed FFT for the
fast plane). The only outstanding neural item remains the one from the first pass:
SWIPE-tiny, if and when noise becomes a measured problem.

### Onset / latency framing
- **Onset detection** — catch the *start* of a note from the energy burst
  immediately, before the pitch stabilises; use it to trigger the note earlier.
  [Cycfi Research](https://www.cycfi.com/2021/01/onset-detection/),
  [comb-filter onsets (arXiv 1611.06505)](https://arxiv.org/pdf/1611.06505).
- **Perceptual latency threshold** for pitch feedback ≈ **30–40 ms**; below that
  reads as instant.

## Pseudo Wigner-Ville distribution (PWVD) — what it is

- **Wigner-Ville distribution (WVD):** a quadratic (energy) time-frequency
  representation — the Fourier transform of the signal's instantaneous
  autocorrelation. It has the *highest* joint time-frequency resolution of any
  TF distribution (no window-length uncertainty tradeoff like the STFT/spectrogram).
- **Its curse — cross-terms:** because it's quadratic, any two frequency
  components create a spurious oscillating "ghost" component halfway between them.
  For a harmonic tone (many partials) this litters the plane with interference.
- **Pseudo WVD (PWVD):** apply a smoothing window (in frequency, and optionally
  time → *smoothed* PWVD) that attenuates those cross-terms, trading a little
  resolution for a readable plane.
- **This paper's contributions (Liu et al. 2022):**
  1. a **faster algorithm to compute the PWVD** (the usual knock on WVD/PWVD is
     cost — this is what makes it "fast");
  2. **cepstrum-based pre-filtering** to knock down cross-term interference before
     the PWVD (cleaner than smoothing alone);
  3. voiced/unvoiced handling + robustness to **abrupt frequency shifts**.
- **Result:** on a speech+electroglottograph corpus, **state-of-the-art MAE ≈ 4 Hz**.
- **Relevance to us:** it's a **speech F0** method (single voice), validated on
  speech, not violin — so not a drop-in. But the *idea* is attractive: the fine
  time-frequency localisation is a low-latency, high-resolution alternative to a
  long YIN window, and the reassigned resonator bank is already our cheap approximation
  of "high-res TF localisation without a big window". Worth a spike **only if** the
  resonator+pYIN path proves insufficient; the cross-term handling on a rich
  harmonic (bowed-string) spectrum is the risk.

## Recommendation for this app (priority order)

1. **Use the resonator peak as a fast onset/pitch prior** to shorten the YIN
   commit delay — cheapest win, all the pieces exist.
2. **pYIN-style HMM smoothing** over pitch candidates to remove octave errors and
   jitter (also the plan's Phase 2 groundwork).
3. **Shorter / asymmetric YIN window** to cut group delay if step 1 isn't enough.
4. CREPE / PWVD — research spikes only if 1–3 fall short; both are heavier and
   speech-oriented / model-dependent.
