# testdata — real violin recordings

Ground truth for the pitch detector. These exist because Phase 1.11 (`dsp::swipe`) had
one risk that could not be reasoned out — whether SWIPE′'s negative valleys fire on the
bank's *spiky* reassigned column, where the paper assumes a dense √-spectrum — and because
this plan's standing rule is that "built, not live-verified" is not done.

They are read by `audio::dsp::swipe::tests::real_violin_g_probe`:

```
cargo test --lib real_violin_g_probe -- --ignored --nocapture
```

`#[ignore]`d because it is slow (it drives the real bank over ~90 s of audio), not because
it is optional.

## How they were recorded

Do this again the same way, or the takes stop being comparable.

- **Instrument/room**: the user's violin, their room, their playing.
- **Mic**: FIFINE K669 (USB condenser), hardware gain **untouched between takes** — the
  measurement is a spectral *balance*, and moving the gain smears it.
- **Path**: `parecord` straight off the PulseAudio source the app itself captures from, so
  the recording is the signal the detector sees.

  ```fish
  parecord --device=alsa_input.usb-3142_FIFINE_K669_Microphone-00.mono-fallback \
           --file-format=wav --rate=44100 --channels=1 --format=s16le <name>.wav
  ```

- **Format**: WAV / 44100 / mono / s16le. **Never a lossy codec** — it discards quiet
  components first, i.e. exactly the weak 196 Hz fundamental this corpus is about.
- **No processing**: no normalisation, EQ, noise suppression, reverb, and above all **no
  AGC** — an auto-gain stage flattens the harmonic balance, which is the only quantity
  being measured. (Phone recorders do this by default. This box has no echo-cancel or
  noise-suppression module loaded; checked with `pactl list short modules`.)

Verified clean on arrival, before any conclusion was drawn from them: no clipping, and
mean 12–20 dB below peak, i.e. dynamics intact rather than compressed.

| take | length | peak | mean |
|---|---|---|---|
| `g_open_slow_strokes` | 9.8 s | −24.6 dB | −37.2 dB |
| `g_open_fast_strokes` | 5.9 s | −18.1 dB | −32.5 dB |
| `g_open_real_octave` | 17.6 s | −18.9 dB | −32.5 dB |
| `g_string_trill` | 17.6 s | −11.7 dB | −31.0 dB |
| `a_string_trill` | 35.3 s | −17.4 dB | −31.9 dB |

## The takes, and what each one is *for*

### `g_open_slow_strokes` — the reported bug

Open G (196 Hz), slow détaché, ~8–10 separate bow strokes. This is the live report
verbatim: *"на струне G — много перескакиваний на фантомную октаву просто от bow
strokes"*.

**Truth: every voiced frame is G3 (MIDI 55).** Anything else is an error.

Why the open G: the violin barely radiates its own G fundamental, so the 2nd harmonic
(392 Hz) is louder than the note. A scorer that requires the fundamental's own bin to
carry energy cannot survive this string — which is precisely what `FUNDAMENTAL_FLOOR = 0.18`
did.

### `g_open_fast_strokes` — the same bug, faster

As above, quick strokes. More attacks per second, so more of the frames are transient.

**Truth: every voiced frame is G3.**

### `g_open_real_octave` — the control that keeps the fix honest

Open G, then deliberately the octave above.

**Truth: G3 *and* a real octave.** This take exists to catch the opposite failure: a
scorer that "fixes" the phantom octave by simply biasing downward would read 0% up here
too, and look perfect on the other takes. Any change must keep reporting the octave in
this take while reporting none in the strokes takes. **That contrast is the test** — one
take alone cannot distinguish a discriminator from a bias.

Note: the played octave came out about a semitone sharp (reported as G#4, ~415 Hz rather
than 392). That is the take, not a detector error — confirmed by the player ("skill
issue"). The app is an intonation trainer; it reported the intonation.

### `g_string_trill` — the case that breaks duration filters

The player putting fingers down in **first position** on the G string — *"я просто тыкал
пальцами в первой позиции"*.

**Truth: every note from the open string to the 4th finger, i.e. MIDI 55–62** (G3 · G♯3 ·
A3 · B♭3 · B3 · C4 · C♯4 · D4). Anything outside that band is an error.

> **Read this before calling a spread an error.** This take was first written up as SWIPE′
> "spreading up to a fourth above the open G", which looked bad — until the player said
> what they had actually played. `A3 · G3 · A♯3 · B3 · C4 · G♯3` *is* first position on the
> G string. The detector was reporting the notes; the probe's G3/G4 buckets were filing
> real content under "other". The corpus is only ground truth to the extent someone says
> what is in it, and a take whose truth is guessed will be used to prove whatever the
> guess was.

Why a trill matters: it is the one thing a duration-based filter cannot survive. A
two-frame trill note and a two-frame octave spike are the same length, so anything that
rejects "short" excursions rejects the trill too. `dsp::octave_gate`'s module doc already
says this — it is why that gate is interval-based, not duration-based.

### `a_string_trill` — the same, one string up

First position on the A string, where the fundamental is *not* suppressed by the body.
Isolates "trill" from "weak fundamental": if this misbehaves too, the cause is the playing,
not the G string's physics.

**Truth: MIDI 69–76** (A4 open .. E5, 4th finger).

The player also notes some readings are likely an **overtone from an under-pressed string**
(*"обертон от C из-за недожима"*) — a finger that does not fully stop the string sounds
partly like a harmonic. That is a real acoustic event in the take, not a detector fault:
the instrument really did emit it.

## Measured — 2026-07-15, at `cfa4f68`

Both scorers on the **same column of the same bank frame**, so the only variable is the
scoring. Tallied over the frames the old scorer itself calls voiced.

| take | OLD comb | SWIPE′ |
|---|---|---|
| `g_open_slow_strokes` | G3 33.7% · **phantom G4 57.2%** · other 9.2% | G3 73.5% · **G4 0.0%** · other 26.5% |
| `g_open_fast_strokes` | G3 **1.4%** · phantom G4 43.3% · other 55.3% | G3 79.0% · G4 0.3% · other 20.7% |
| `g_open_real_octave` | G3 7.8% · G4 47.3% · other 44.9% | G3 41.6% · **G4 14.2%** · other 44.2% |
| `g_string_trill` | G3 4.6% · G4 4.8% · **other 90.6%** | G3 17.5% · G4 0.6% · other 81.9% |
| `a_string_trill` | — · — · other 100% | G3 0% · G4 0.2% · other 99.8% |

What SWIPE′ actually reports, most common first:

```
g_open_slow_strokes : G3 74% · G2 4% · D#1 3% · E1 2% · F#0 2% · F#1 1%
g_open_fast_strokes : G3 79% · G2 3% · G0 1% · A#0 1% · F0 1% · A0 1%
g_open_real_octave  : G3 42% · G#4 29% · G4 14% · G#3 5% · F#0 1% · F0 1%
g_string_trill      : A3 19% · G3 17% · A#3 15% · B3 15% · C4 11% · G#3 5%
a_string_trill      : A4 26% · B4 21% · C5 11% · A#4 10% · C#5 4% · F#0 2%
```

### What this establishes

- **The phantom octave is gone on the strokes takes** — 0 of 612 frames, against a shipped
  scorer that called the phantom *more often than the truth* (57.2% vs 33.7%) and, on the
  fast take, was right in **1.4%** of frames.
- **It is a discriminator, not a downward bias** — the octave take still reads G4 at 14.2%.
- **§4.3 is answered**: the valleys fire on a real spiky reassigned column.

### What this does NOT establish, and must not be claimed

- **Only the G and A strings, and only these five takes.** D and E are unrecorded. The
  repair layer above the bank (`snap_to_anchor_octave`, `LEAP_CONFIRM_FRAMES`,
  `OctaveGate`) is still in place *because* of this gap — retiring it on one string's
  evidence would be this plan's own recurring mistake.
- **The trills are unexplained** (below).

Scored against the real first-position bands, which is the number that means something:

| take | in position | error | **of which an octave of a played note** |
|---|---|---|---|
| `g_string_trill` | 83.8% (924/1103) | 16.2% | **7.5%** |
| `a_string_trill` | 76.9% (1697/2206) | 23.1% | **5.3%** |

**This confirms the live report on the current scorer, not just the old one.** The player
reported *"на pitch roll постоянно промахи через октаву вверх и низ"*, and octave errors
are still here at 5.3–7.5% of frames — an order of magnitude better than the old comb, but
"better" is not "right", and a pitch roll draws every frame. The remaining ~9–18% is mostly
the low-register junk below.

## Open questions

1. **The octave errors that survive SWIPE′** (5.3–7.5% on the trills). These are *lone*
   excursions from a correct neighbourhood, which is exactly what per-frame argmax cannot
   fix and what temporal continuity can: a Viterbi over the salience curve outvotes a
   one-frame jump without needing a rule about octaves. Note the trap in stating it as
   "a jump of an octave or more is an error" — `g_open_real_octave` is the player
   deliberately leaping an octave, and that rule would break it. The distinction is not
   the interval, it is whether the evidence *persists*.
2. **Low-register junk.** The strokes takes report G2 (~4%) and scattered C0–E1; the trills
   show F♯0. The bank offers candidates down to C0 = 16 Hz where no violin plays. That is a
   candidate-range question and must not be answered with a violin-shaped magic floor.
3. **A trill is polyphonic in the bank, and the scorer is not.** At a trill's rate the
   previous note has not decayed when the next starts — ring-down is ~80 ms at G3
   (constant-Q; see `resonator::bank_latency_probe`) — so the column genuinely holds *two*
   harmonic series, and SWIPE′ picks the single series that best explains it, which for a
   mixture need not be either note. Hypothesis with a mechanism, not a finding. It predicts
   a worse in-position rate on G (longer ring) than on A; the takes say **83.8% G vs 76.9%
   A**, i.e. the *opposite*, so either the mechanism is not dominant here or the A take is
   harder for another reason (it is twice as long and reaches higher). Unresolved.
