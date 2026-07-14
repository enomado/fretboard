# SWIPE′ salience over the resonator bank — design

Design for replacing `analysis_math::resonator_fundamental` — the melody line's octave
decision — with a real SWIPE′ pitch-strength function computed over the resonator
bank's column.

> **Status: designed, NOT built.** This file is the design and its justification.
> [`note_detection.md`](note_detection.md) is the canon for what actually runs; it
> does not mention any of this yet, and must not until this ships.

Companion to [`pitch_detection_survey.md`](pitch_detection_survey.md) (which already
listed SWIPE′ but never explained *why* it is the answer here) and to the plan's
Phase 1.11.

## 1. The defect, stated exactly

`resonator_fundamental` scores a candidate bin `b` as

```rust
for h in 1..=RESONATOR_HARMONICS {          // 5 harmonics
    score += column[b + offset(h)] / h;      // offset(h) = bins_per_semitone·12·log2(h)
}
```

guarded by `if column[b] < FUNDAMENTAL_FLOOR { continue }` (floor = 0.18).

This is a **reward-only harmonic comb**: it adds credit at harmonics and can never
subtract. Camacho's thesis enumerates exactly this function and exactly its failure
(Figure 3-13, panels C/D/E):

> **C) Scores using only positive cosine lobes (exhibits peaks at sub and
> supraharmonics)**
> **D) Scores using both positive and negative cosine lobes (exhibits peaks at
> subharmonics)**
> **E) Scores using both positive and negative cosine lobes at the first and prime
> harmonics (exhibits a major peak only at the fundamental)**

**We are panel C.** Our phantom octave *is* the supraharmonic peak. Two independent
devices fix it, and we have neither:

- **negative valleys** kill the *supra*-harmonic (our bug);
- **first-and-prime harmonics** kill the *sub*-harmonic (SWIPE′ proper).

### 1.1 Why the G string specifically

The violin barely radiates at 196 Hz — the open G's fundamental is far weaker than its
2nd harmonic. Camacho's own worked example for choosing the spectral warping is a vowel
with *"a missing fundamental and a salient second harmonic"*, whose fundamental sits at
**190 Hz** and 2nd harmonic at **380 Hz**. The violin G is 196/392. SWIPE was designed
on our signal.

Take candidate `f = 392` (the phantom G4) against a true G3 = 196:

| G3 harmonic | Hz  | `f'/f` vs 392 | SWIPE kernel lobe   |
|-------------|-----|---------------|---------------------|
| h1          | 196 | 0.5           | **negative** (halved) |
| h2          | 392 | 1.0           | positive            |
| h3          | 588 | **1.5**       | **negative**        |
| h4          | 784 | 2.0           | positive            |
| h5          | 980 | **2.5**       | **negative**        |

G3's *odd* harmonics land precisely in G4's valleys and are subtracted. The thesis:

> The positive cosine lobes at the harmonics of 200 Hz produce a positive contribution
> towards the score of the 200 Hz candidate, **but the negative cosine lobes at the odd
> multiples of 100 Hz cancel out this contribution.** … The effect on the 200 Hz peak
> is definite: **it has disappeared.**

Meanwhile candidate `f = 196` scores from 196, 392, 588, 784 … — *all* positive lobes —
**even when the 196 bin itself is empty**. The score comes from the harmonics, not from
the fundamental's own bin. That property is the whole answer to the G string, and it is
the exact property `FUNDAMENTAL_FLOOR` destroys.

### 1.2 `FUNDAMENTAL_FLOOR` is a crutch standing in for the valleys

Its comment says *"the fundamental itself must carry energy — this is the sub-octave
guard"*. That is honest about its job: a reward-only comb peaks at subharmonics
(panel C), so something had to block them, and the floor was it. SWIPE′ does that job
properly, with primes. Once the kernel is right, **the floor has no job left** — and it
is the floor that is killing the G string, because it is a hard cliff standing exactly
where a weak fundamental lives.

### 1.3 The second root cause: on a bow stroke the anchor is an echo of the bank

**The bank's phantom octave explains the error. This explains why nothing catches it.**

`OnsetDetector` is *designed* to fire on every bow stroke — its module doc: *"a
re-articulated note (**a new bow stroke** on the same pitch — a dip then a rise, with no
pitch change) dips below the baseline and re-arms, so its second attack fires too"*. That
is deliberate and correct; the staff needs it to split repeated same-pitch notes.

But `pyin::process` on an onset frame (`pyin.rs:293-309`):

```rust
let weight = if onset { ATTACK_BANK_WEIGHT } else { BANK_WEIGHT };  // 2.0 vs 0.5
candidates.push(Candidate { frequency_hz, probability: weight * strength });
…
if onset { self.initialized = false; }                               // trellis dropped
```

With the trellis dropped the frame is decided by **emissions alone**, and the bank at
`2.0 × strength` beats YIN's own candidate, which measures `p = 1.000`. **So on a bow
stroke, pYIN's output is the bank's pitch.**

Then the whole repair layer folds:

| stage | what it does on a bow stroke |
|---|---|
| bank | says G4 (the panel-C defect, §1.1) |
| pYIN anchor | **echoes G4** — trellis dropped, bank weighted 2.0 |
| `snap_to_anchor_octave` | sees *agreement*, not dispute → snaps to G4 |
| `LEAP_CONFIRM_FRAMES` | never counts — `octave_dispute` is reset by agreement |
| `OctaveGate` | 5-frame median follows the run → accepts |

Every layer built to catch the bank's octave error is, on exactly the frames where that
error fires, fed by the error itself. The anchor is only an anchor while it is
*independent of the bank*, and on onset frames it is not.

**The docs are wrong about this and say so confidently.** `melody.rs`: the fusion
*"provably contributes nothing"*; the plan: *"harmless"*, *"inert"*, *"dead weight"*. That
was measured for `BANK_WEIGHT` with the trellis **intact** — where a capped 0.5 cannot
outvote `p = 1.000` and the continuity bias buries it besides. `ATTACK_BANK_WEIGHT` with
the trellis **dropped** is a different regime; the measurement does not transfer, and the
doc generalised it anyway. This is the plan's own trap (Phase 1.4–1.6) in miniature: a
number verified in one condition, restated as a property.

**`ATTACK_BANK_WEIGHT` must die, and not as cleanup.** Even after SWIPE′ makes the bank
right, an anchor that echoes the bank is not evidence. Kill the fusion first — it is
independent of this design, cheap, and it restores the anchor's independence whatever else
we do. Note the in-flight `fusion_probe` in `pyin.rs` is measuring exactly this.

### 1.4 A display slider currently steers the octave decision

`normalize_bars(&mut spectrum, settings.gamma)` runs at `resonator.rs:337` and its
output goes straight into `resonator_fundamental` at `:338`. `settings.gamma` is a
**user-facing waterfall-contrast slider** (`controls.rs:763`, range `0.15..=2.4`).
`settings.power` likewise squares the spectrum, which the thesis identifies as the worst
of the three warpings tested.

So today the pitch detector's behaviour is a function of a display knob. This is the
same disease as Phase 1.9's silence gate sharing the UI level meter's smoothing — a UI
filter steering DSP — and this design must not carry it forward: **the detector takes
its own warped copy of the column and never reads `gamma`/`power`.**

## 2. The kernel, exactly (thesis eq. 3-12)

```
                ⎧ cos(2π f'/f)        , if 3/4 < f'/f < n(f) + 1/4
K(f, f')   =    ⎨ ½ cos(2π f'/f)      , if 1/4 < f'/f < 3/4  or  n(f)+1/4 < f'/f < n(f)+3/4
                ⎩ 0                   , otherwise
```

with `n(f) = ⌊f_max/f − 3/4⌋`. Pitch strength (eq. 3-11) is the inner product of the
kernel with **√|X|**, under a `1/√f'` envelope, normalised by the L2 norm of the
kernel's **positive part only**:

```
              Σ  (1/√f') · K(f,f') · √|X(f')|
strength(f) = ───────────────────────────────
                ( Σ (1/f') · [K⁺(f,f')]² )^½
```

Three details that are load-bearing and easy to lose:

- **√ of the spectrum, not the magnitude and not the power.** *"Panel D shows the
  square-root of the spectrum, which neither overemphasizes the missing fundamental (as
  the logarithm does) nor the salient second harmonic (as the square does)."* Also: with
  √, each harmonic's contribution to the inner product is proportional to its
  *amplitude* rather than its square.
- **Harmonic decay `1/k^p` with `p = 1/2`**, realised by the `1/√f'` envelope. The thesis
  tested `p ∈ {1/2, 1, 2}` and found `p = 1/2` best. **Our `1/h` is `p = 1` — the one
  they rejected.**
- **The DC lobe is truncated** (kernel is 0 below `f'/f = 1/4`) and the **first and last
  negative lobes are halved**, to avoid bias.

### 2.1 SWIPE′

Keep only lobes at `h ∈ {1} ∪ primes`:

```
K(f, f') = Σ_{i ∈ {1} ∪ P} K_i(f, f')
```

Rationale, verbatim: *"if there is a match between one of the prime harmonics of this
candidate and a harmonic of 100 Hz, no other prime harmonic of the candidate can match
another harmonic of 100 Hz, and therefore the score of all the candidates below 100 Hz
has to be low"*. Valley weights change with the peaks removed: only the valley between
h1 and h2 and the valley between h2 and h3 keep weight −1; **all other valleys weigh
−1/2** (before the decay envelope), because each valley is shared by two peaks and one
of them is now gone.

### 2.2 The claim that matters for us

> Except for the obvious architectural decisions that must be taken when creating an
> algorithm (e.g., selection of the kernel), **there are no free parameters in SWIPE and
> SWIPE′, at least in terms of "magic numbers".**

and

> **SWIPE′ was shown to outperform all the algorithms on all the databases.**

(12 competitors; speech + musical-instrument databases.)

## 3. The reformulation: on a log axis the kernel is one fixed vector

This is the design's key idea and the reason it fits this app rather than fighting it.

`K` depends on `f'` and `f` **only through the ratio `r = f'/f`**. Our bank's output grid
is uniform in log-frequency (`OUTPUT_BINS_PER_SEMITONE = 8`, C0..C8). Put
`u = log₂(f'/f)` in octaves. Then:

- **Lobe positions** are at `u = log₂(h)` — independent of `f`. (The current code already
  exploits this: `offset(h) = bins_per_semitone·12·log2(h)`.)
- **Lobe edges** are at `u = log₂(h ± ¼)` — independent of `f`.
- **The envelope** `1/√f' = (1/√f)·2^(−u/2)`; the `1/√f` factor is constant per candidate
  and cancels in the normalised ratio, leaving the shape `2^(−u/2)` — independent of `f`.

Therefore **`G(u) = 2^(−u/2)·K(2^u)` is a single fixed vector**, and

```
strength(f) ∝ Σ_u G(u) · S(u_f + u)          where S = √column, u_f = candidate's bin
```

i.e. **the entire salience curve over all candidates is one cross-correlation of a fixed
kernel with the √-warped column.** Consequences:

- No per-candidate kernel construction. Build `G` once at startup.
- The whole salience curve comes out at once — which is exactly the input an online
  Viterbi would want later, if we ever decide the fast channel needs temporal smoothing.
  (Not part of this design. Possibly unnecessary once the salience stops degenerating.)
- If naive correlation proves too costly, it is an FFT away — `rustfft` is already a
  dependency.

### 3.1 Grid resolution bounds the harmonic count — no magic number needed

Lobe `h` spans `12·log₂((h+¼)/(h−¼))` semitones. At 8 bins/semitone:

| h  | lobe width (semitones) | bins |
|----|------------------------|------|
| 1  | 8.84                   | 71   |
| 2  | 4.35                   | 35   |
| 3  | 2.89                   | 23   |
| 5  | 1.73                   | 14   |
| 7  | 1.24                   | 10   |
| 11 | 0.79                   | 6.3  |
| 13 | 0.67                   | 5.3  |
| 17 | 0.51                   | 4.1  |

Past `h ≈ 13` a cosine lobe is sampled by ~5 bins and the kernel stops being represented
honestly. So the harmonic set falls out of the grid rather than being picked: **`{1} ∪
primes ≤ 13`**, i.e. `{1, 2, 3, 5, 7, 11, 13}`. If we want more, the answer is a finer
grid, not a bigger constant.

### 3.2 Truncation must be normalised, not `break`-ed

The current loop does `if idx >= column.len() { break }` and then compares raw scores.
A candidate near the top of the range therefore sums *fewer* harmonics than a low one and
scores lower for a reason that has nothing to do with the signal — a systematic bias
toward low candidates, i.e. toward the sub-octave. (Plausibly part of why the floor guard
felt necessary. Unverified.)

SWIPE's denominator handles exactly this: normalise by the norm of the **truncated**
positive part. Cheap here — precompute the prefix sums of `[G⁺]²` and take a partial sum
per candidate.

## 4. Where we deviate from SWIPE, and why

Stated up front, because each is a place this could be wrong.

1. **ERB axis → log-frequency axis.** The thesis picks ERB for a stated *computational*
   reason: *"the density of this scale decreases almost proportionally to frequency,
   which avoids wasting computation in regions where little spectral energy is
   expected."* A log axis has that property exactly, and for music it is the natural one
   — it is also what makes §3's shift-invariance work. But the measure change (`dε` → `du`)
   reweights low against high harmonics slightly. **Believed benign; not verified.**
2. **No per-candidate window.** SWIPE needs pitch-dependent window sizes (`4k/f`) and
   blends two powers of two, because it computes its own FFT. We don't: the resonator
   bank already *is* a per-candidate filter with its own time constant. This is a genuine
   structural advantage, not a shortcut — but our bins are not tuned to SWIPE's optimal
   `4k/f`, so the match to the ideal kernel is approximate. **Extent unquantified.**
3. **The column is not a dense spectrum.** Reassignment splats energy to its measured
   instantaneous frequency and the coherence gate drops what does not cohere, so the
   column is spiky where a real √-spectrum has skirts. The valleys' punishment still
   fires — for candidate 2f0 the valleys sit on 3f0/5f0, where reassigned spikes really
   are — so the mechanism should survive, and possibly sharpen. **But this is the
   biggest risk in the design**: a spike landing on a lobe's zero-crossing contributes
   nothing, and the coherence gate may already have removed energy SWIPE expects to see.
   This is the thing to measure first.
4. **Our own warping.** `√` (p = 0.5) computed by the detector from `bank.magnitudes()`,
   never from the display's `gamma`/`power`. See §1.4.

## 5. What this deletes

By construction, not by tuning — which is the point:

| dies | why |
|---|---|
| `FUNDAMENTAL_FLOOR` | its job (sub-octave) is the primes' job now |
| `RESONATOR_HARMONICS = 5` | harmonic set falls out of the grid (§3.1) |
| `1/h` weighting | replaced by the envelope, `p = 1/2` |
| detector's use of `gamma`/`power` | detector warps its own copy |

Separately and **first**, independent of this design (§1.3):

| dies | why |
|---|---|
| `ATTACK_BANK_WEIGHT = 2.0` | makes the anchor echo the bank on every bow stroke |
| `BANK_WEIGHT`, `BANK_FUSE_LEVEL` | the rest of a fusion that was never load-bearing |

And, **pending verification that the fast channel stops lying about the octave**, the
entire repair layer above it:

| dies | why |
|---|---|
| `snap_to_anchor_octave` | nothing left to snap |
| `YIN_OCTAVE_CONFIDENCE`, `OCTAVE_AGREE_SEMITONES` | ditto |
| `LEAP_CONFIRM_FRAMES = 4` | the plan admits it was *"picked from an argument"* |
| `OctaveGate` (`OCTAVE_REJECT`, `MEDIAN_WINDOW`) | last-resort slip rejection with no slips |

That is ~7 hand-picked constants, and it removes the melody path's dependency on the slow
pYIN anchor entirely. pYIN stays where it belongs: the tuner and the fretboard.

**Do not delete these speculatively.** Land the kernel, verify (§7), then remove the layer
above it in a separate commit so the two are independently revertable.

## 6. Placement

A real algorithm deserves its own module: **`audio::dsp::swipe`**, not another function in
`analysis_math` (which is a bag of helpers). It owns the kernel construction, the prefix
sums, and the salience. `resonator_fundamental` disappears; `resonator_snapshot` calls the
new module and keeps `normalize_bars` for the *display* copy only.

## 7. How to verify this WITHOUT the instrument

The plan's standing rule is that "built, not live-verified" is not done, and three phases
are already sitting in that state. This one does not have to join them — the defect is
offline-reproducible, which is what makes it a good next move.

**The golden test that fails today.** The existing test
`resonator_fundamental_picks_fundamental_over_louder_overtone` uses a two-partial spectrum
(`col[A4] = 0.6`, `col[A5] = 1.0`) and passes — it is far too easy, and its passing is why
nobody noticed. A violin G is not two partials:

```
G3 = 196 Hz, harmonic amplitudes ≈ [0.08, 1.0, 0.7, 0.5, 0.4, 0.3, ...]
                                     ↑ the body does not radiate here
```

Assert the salience peaks at **G3**, not G4. Today's code cannot pass this: at 0.08 the
fundamental is below `FUNDAMENTAL_FLOOR` after warping, so G3 is never even *considered* a
candidate — the test fails at the guard, before any scoring. That is the defect made
executable.

Then, in order:

1. **Sweep the fundamental's amplitude** 0.0 → 1.0 with the harmonics fixed, and assert the
   decision never leaves G3 — including at **exactly 0.0** (missing fundamental). The
   current code's answer flips somewhere in that sweep; SWIPE′'s must not. This single
   test *is* the "by construction" claim, made falsifiable.
2. **Sweep `gamma` across its full slider range** (0.15..2.4) and assert the octave decision
   does not move. Fails today by construction (§1.4); must pass after.
3. **Bow-stroke simulation**: modulate the harmonic balance over time (h2 louder then
   quieter, as a bow change does) and assert the decision is constant. This is the
   user's actual report, offline.
4. **Re-run `bank_latency_probe`** — the salience must not cost latency. This design does
   not add temporal state, so it should be unchanged; assert it rather than assume it.
5. Only then the instrument, for what tests genuinely cannot answer: does a real G string
   with a real bow hold.

## 8. Open questions — decide before cutting

- **§4.3 (sparsity vs valleys)** is the one that could sink this. Measure before building
  the rest: take a real reassigned column, place the SWIPE′ kernel at 2·f0, and check the
  valleys actually collect the odd harmonics' energy.
- Does the salience replace `resonator_fundamental`'s `strength` return (used by the
  silence gate) or do we keep a separate level? The salience's peak value is a *pitch
  strength*, not a level — probably not interchangeable, and the release-ghost bug (plan
  §1.9) already lives in that gate.
- SWIPE′ has no Rust implementation on crates.io (`pitch-detection`, `pyin-rs`,
  `autopitch` — none carry it), so there is no reference to diff against. A golden oracle
  is worth the trouble here: Camacho's MATLAB/Octave `swipep.m` is public; running it on
  the same synthetic columns gives a non-circular check.

## Sources

- Camacho, A. *SWIPE: A Sawtooth Waveform Inspired Pitch Estimator for Speech and Music.*
  PhD thesis, University of Florida, 2007. Figure 3-13 (panels C/D/E), §3.3–3.5 (warping
  and harmonic weighting), eq. 3-11/3-12 (strength and kernel), §3.9 (SWIPE′), Chapter 5.
  <https://kerwa.ucr.ac.cr/server/api/core/bitstreams/fcb3aea1-3a00-421d-9dc4-dd7e9c2e915c/content>
- Camacho, A. & Harris, J. G. *A sawtooth waveform inspired pitch estimator for speech and
  music.* JASA 124(3):1638–1652, 2008.
  <https://pubs.aip.org/asa/jasa/article-abstract/124/3/1638/676279/A-sawtooth-waveform-inspired-pitch-estimator-for>
- SWIPE′ summary, PsySound3. <http://psysound.wikidot.com/system:swipep-pitch-estimation>
- *Improving Neural Pitch Estimation with SWIPE Kernels*, 2025 — SWIPE kernels still used
  as a front end. <https://arxiv.org/html/2507.11233v1>
