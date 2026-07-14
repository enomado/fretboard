# Pitch-detection algorithm survey — for the violin trainer

Notes for "какая нота играет сейчас" + latency work. Companion to
[`violin_trainer_plan.md`](violin_trainer_plan.md). What this app already has, what
the literature offers, and the concrete next steps.

## What the app already does

> **Status note (Phase 1.7).** The "wins come from marrying them" line below turned
> out to be exactly right, and the numbers here were confirmed by measurement — see
> `violin_trainer_plan.md` Phase 1.7. The marriage only works with the **bank
> leading** and YIN pinning the octave (`audio::dsp::melody`). The other
> arrangement — fusing the bank into the pYIN HMM as a weighted candidate — was
> built (Phase 1.5) and measured to contribute **exactly zero**, because YIN's own
> candidate scores `p = 1.000` against the bank's capped `≤ 0.5`.

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

### Data-driven (accurate, heavy)
- **CREPE** — Kim, Salamon et al., 2018. CNN on the raw waveform; beats pYIN on
  accuracy, authors cite <10 ms. **But** needs a trained model + inference — a poor
  fit for pure Rust / egui / wasm right now. Park for later.
  <https://www.semanticscholar.org/paper/Crepe:-A-Convolutional-Representation-for-Pitch-Kim-Salamon/86aeec4d48d949190b3a0c2bf32c101fc23f13a3>
- Benchmarks / comparisons: [pitch-benchmark (lars76)](https://github.com/lars76/pitch-benchmark),
  [Comparing PDAs vs data-driven (arXiv 2206.14357)](https://arxiv.org/pdf/2206.14357),
  [RT F0 via spectrogram+CNN (arXiv 2504.06165)](https://arxiv.org/pdf/2504.06165).

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
