# Violin Trainer — multi-session plan

A living roadmap for the notation-based violin trainer. Update this file as phases
land; it is the hand-off between sessions. Keep phase status honest (done / in
progress / not started) and note the commit when a phase ships.

> **"✅ DONE (built, not live-verified)" is not done.** Phases 1.4–1.6 all carried
> that label, and all three shipped a latency regression nobody caught for weeks —
> see Phase 1.7. Either verify it, or write a test that measures the thing you are
> claiming, or say plainly that it is unverified in the status itself.
>
> **The mechanism is described in [`note_detection.md`](note_detection.md)** — read
> that before touching pitch, octaves, note starts/ends or latency. This file is the
> history; that one is the current shape, with the measured numbers.

---

## ▶ Start here — handoff (2026-07-14, third session of the day)

### Where it stands

**The instrument has not been played since `1a8c6e1`.** Three phases have landed on top
of it. Read [`note_detection.md`](note_detection.md) before touching any of them — it is
the whole mechanism in one place, and the checklist below assumes it.

| commit | what | live-verified? |
|---|---|---|
| `1a8c6e1` | melody rides the resonator bank again; latency measured | ✅ **yes** — "работает неплохо" |
| `969e37b` | universal ceiling (C8), quantile framing, one octave decision | ❌ **no** |
| `a063606` | systematisation doc; panel range clamps dropped | ❌ **no** |
| `925a2a0`, `d4d668b` | UI design tokens (chrome colour only — no DSP) | ✅ render only |
| *(this session)* | **1.9** — segmentation off the UI clock; silence gate into the engine | ❌ **no** |

**That table is the most important thing on this page.** The user's "работает неплохо"
was given *after `1a8c6e1` and before everything below it*. All of it is
tests-and-reasoning only — which is the exact state Phases 1.4–1.6 were in when they
shipped a 100 ms regression that took weeks to notice.

> **⚠ The trap, twice now, and it very nearly worked both times.** While screenshotting
> the *token* render, the user said **"я проверил всё ок"**. That sentence is about
> **chrome colour**. It is not a verification of 1.8: octave stability on sustain, cents
> resolution and the new C6..E7 ceiling all need the instrument, and none of them was
> played. A short approving sentence arriving mid-session is not evidence for whatever
> else happens to be in the tree. If you are about to tick a phase off, ask what was
> actually played.

### 🐞 Read this before the next live session

**There is a known bug that will dominate it if you don't know about it.** After every
note stops, the staff writes ghost notes nobody played — measured: A4 → a C#0 (MIDI 13)
with a ledger stack. The silence gate closes ~500 ms late (it shares the UI level
meter's smoothing) and the bank, whose column is normalized, spends that window
reporting whichever bins ring longest at full confidence. Full diagnosis in Phase 1.9;
pinned by `core::release_ghosts_are_written_after_a_note`.

It is **pre-existing and not caused by 1.9** — 1.9's new engine-wiring test is simply
the first thing that ever looked. It is deliberately **not fixed**: the likely fix
(give the melody gate the *raw* window level instead of the meter's smoothed one) is a
DSP behaviour change that needs its own live verification.

**But the test cuts the tone off instantly, and no instrument does.** A real note decays
over hundreds of ms, so the gate and the bank may well fall together and this may be
invisible in practice. **That is the first question to put to the instrument**: play a
note, stop, and look — do ghosts appear below the staff? The answer decides whether the
fix is urgent or unnecessary.

### The UI track (finished this session)

Chrome colour only — **no DSP, no behaviour**. Panels name a role
(`color::TEXT_MUTED`) instead of a value; `ui::tokens` is the vocabulary and its
module docs are the spec. Two things there are worth knowing before touching UI:

- **`pill()` takes no colours.** Its default badge is the signature. It used to take
  `fg`/`bg`, and half its twelve call sites hand-passed near-identical greys until one
  chip style had drifted into four. Use `pill_muted` for the empty state and
  `pill_colored` only when the colour *is* data (the staff's intonation chip).
- **Token if a role is spoken in ≥2 modules; a named `const` in the file if in one.**
  A global name used once dilutes the vocabulary. Promote when a second module needs
  it.

Left open **on purpose** — these move pixels on stock egui widgets, which is a design
call and not a mechanical one: three near-identical accents (`ACCENT_FILL`
`(112,86,72)` / `SELECTION_FILL` `(121,92,74)` / `WIDGET_ACTIVE_FILL` `(116,89,73)`),
two idle fills (`IDLE_FILL` vs `WIDGET_IDLE_FILL`), two radii (`CARD` 18 vs `PANEL`
22). They are named at their current values, so nothing is silently wrong.

### Live-verify checklist (next session, with the instrument)

In rough order of risk:

1. **Octave stability on sustain.** Highest risk. `melody::LEAP_CONFIRM_FRAMES = 4`
   (≈64 ms) was picked from an *argument* — that the bank's wandering is intermittent
   while a real leap's disagreement is unbroken — not from live data. If octaves
   flicker on held notes, raise it; if octave leaps feel sluggish, lower it. If
   wandering turns out to be *sustained* rather than intermittent, the whole
   discriminator is wrong and needs rethinking, not tuning.
2. **Cents.** Phase 1.3 flagged this as the one thing YIN did better: the bank's grid
   is ~12.5 cents/bin + sub-bin refine + Δφ. Intonation feedback is the point of the
   panel, so if cents feel coarse, take *cents only* from the anchor and keep timing
   and octave from the bank.
3. **High notes.** C6..E7 are newly reachable and have only ever been tested on
   synthetic tones. Does the bank actually hold a real violin's E string up in the high
   positions?
4. **CPU.** The bank went **360 → 480 resonators** (+33%) with the C8 ceiling, and it
   is per-sample IIR. Watch this on Android especially. `ResonatorSettings::max_midi`
   is user-adjustable, so the escape hatch exists.
5. **Pitch roll framing.** Does the view sit tight and calm, with notes named rather
   than only octaves? Constants: `FRAMING_FRAMES`, `VIEW_QUANTILE_*`,
   `VIEW_SHRINK_SLACK`.

### Next work, in order

1. **Answer the ghost question with the instrument** (see above), then fix or drop it.
   It is the only thing here that is *known* to write wrong notes.
2. **The pitch roll's time axis** — the half of §4 that 1.9 deliberately left. Its
   history is still sampled per UI frame ("~10 s at 60 fps, ~20 s at 30 fps"), so the
   axis is the wrong ruler. This is *display fidelity*, not a decision: the line's
   values are decided upstream and merely sampled late. Not done with the segmenter
   because it needs its own design — an engine-side history the panel drains **by
   sequence** rather than per frame, plus a decision about the heat, which must stay
   aligned 1:1 with the line and costs ~70 MB/s if 600 columns × ~480 bins ride every
   reading. The staff's trail has the same shape and the same excuse.
3. **`LOWEST_TRACKED_FREQUENCY = 16 Hz`** makes `cmndf` search lags to 3000 samples —
   **13.9M ops/frame** for a band no instrument here plays, and the low tail is where
   the sub-bass ghost lives. Cheap win; needs a decision on the real floor.
4. **The inert `BANK_WEIGHT`/`ATTACK_BANK_WEIGHT` fusion** still runs in `pyin`. It is
   harmless and marked "do not fix latency with this", but it is dead weight and it is
   what made Phase 1.5 look reasonable in the first place. (`BANK_FUSE_LEVEL` dies with
   it — it is a third copy of the same `0.02` the melody gate uses.)

### Landmines

- **The bank is park-gated.** Any panel reading `melody_pitch` must call
  `AudioEngine::request_resonator()` every frame or it sits at "play a note…" forever.
  YIN runs unconditionally; the bank does not.
- **`audio::dsp` is private, in every build** (see `audio/mod.rs`) — 1.9 closed the
  `cfg(test)` hole, because the probe no longer has to reach across. Production code
  reads finished values off `TunerReading`.
- **`MelodyTracker::update` and `NoteSegmenter::update` must each be driven at the
  bank's cadence, once per bank frame, from `publish_resonator_snapshot` and nowhere
  else.** The tracker's hysteresis, the gate's median and the segmenter's note timers
  are all counted in those frames. The 40 ms pYIN path deliberately re-stamps the last
  values instead of recomputing — driving either from there too would double-count.
- **The sample clock stalls while the bank is parked** (it counts *processed* samples,
  and a parked bank drops them). Deliberate: nothing downstream runs while parked
  either, and a clock that ran on regardless would expire the held note of whatever was
  playing when the panel was closed.
- **The binary is `fretboard-app`, not `fretboard`** (`[[bin]]` in `Cargo.toml`; the
  lib keeps the crate name so the wasm build can disambiguate). `target/debug/fretboard`
  does not exist and never will.
- **Do not drive the user's live sway session** to verify a render — `swaymsg cursor`
  and window focus steal their desktop out from under them. Launch, `grim` the window
  rect, and ask them to click; they are sitting right there.

## Vision

A practice panel that renders **standard notation** (treble clef staff) and, while
you play the violin, **writes what you play** onto the staff in real time — with
**intonation feedback** so you can see how in-tune each note was. It grows from a
passive "mirror" (see what you played) into an active trainer (play *this*, get
scored).

Pitch source is `TunerReading::melody_pitch` — the resonator bank's fast fine pitch
with its octave pinned by pYIN (`audio::dsp::melody`). **Not** `frequency_hz`: that is
pYIN alone, which is the right tool for the tuner (steady) and the wrong one for a
trainer (it cannot follow a note change in under ~128 ms). See
[`note_detection.md`](note_detection.md) §1.

## Architecture (established Phase 1)

- **`src/ui/staff.rs`** — *stateless* staff renderer (pattern-matches the other
  `ui::*` modules). Pure geometry (`note_staff_step`, `ledger_steps`,
  `treble_clef`, `StaffGeom`) is unit-tested and reused by an offscreen PNG
  preview test (`render_staff_preview`, `#[ignore]`). Vertical position is in
  **diatonic staff steps** (letter-based, so spelling / `AccidentalStyle` matters).
- **`src/app/staff_panel.rs`** — `StaffTrainer` and `App::draw_staff_card`. Held on
  `App.staff`; not persisted. Since Phase 1.9 it holds only what is genuinely the
  panel's own — clef, key signature, and the decorative trail. The note history and
  the glitch/hold/release machine that used to live here are
  `audio::dsp::segmenter::NoteSegmenter`, on an audio clock.
- Wired as `WorkspaceTab::Staff` ("Violin Staff") in `src/app.rs` + dispatch in
  `src/app/workspace.rs`. Opens from the **Panels** menu.
- No music font is loaded — clef is a hand-tuned vector; accidentals are plain
  `#`/`b` text. (See Phase 5 for upgrading to a real SMuFL font.)

Key invariant: `ui::staff` owns *no* state and paints from borrowed inputs; all
trainer state lives in `app::staff_panel`.

## Pitch Roll panel (simpler sibling of the staff)  ✅ built + live-verified

A second, deliberately simpler view added alongside the staff: a horizontal
piano-roll "waterfall table". The staff's latency/octave work (Phases 1.3–1.6) is
premature for the core idea, so this panel just *mirrors* the pitch — no capture
state machine, no quantised noteheads.

**Two layers, chosen deliberately (a live test drove it).** A single pitch *line*
can't be both fast enough to show trills and octave-stable — those pull opposite
ways, and a first cut using the fast bank pitch (`fast_pitch`) plus a threshold
octave-reject filter was a hack the user rightly rejected ("won't get better without
manual tuning"). The fix by construction is to stop forcing one decision:

- **Spectral heat (ground truth, no decision).** The resonator bank's per-column
  energy is painted at each bin's own pitch. It makes *no* single-pitch choice, so
  there is no octave error to make: a strong overtone is just a fainter cell an
  octave up (physically real), the fundamental the bright low cell. Fast (per bank
  column) → trills/vibrato show at full resolution. This is literally the "waterfall
  table" originally asked for.
- **Melody line (a smooth guide on top).** The fused pYIN `frequency_hz`,
  octave-stable and smoothed, coloured by intonation — reads the melody as one clean
  curve. The heat underneath is the backstop: where the line's smoothing or a rare
  octave slip disagrees with reality, the heat shows it.

Files:
- **`src/ui/pianoroll.rs`** — *stateless* renderer. Grid = **one row per semitone**
  labelled with its note name (C-rows bold as octave anchors, accidental rows shaded
  like piano black keys), time **right → left**. Layers bottom→top: rows → heat
  (one [`Mesh`], `HEAT_GATE` sparsity, cool-blue ramp, silence = empty column = gap)
  → intonation line (`theme::intonation_color`, shared with the staff). The **right
  gutter** repeats the note scale, each label coloured by the energy at that pitch
  in the *current* heat column (`draw_right_scale` / `label_heat_color`) — a live
  per-note "what's sounding now" meter at the playhead.
- **`src/app/pitch_roll_panel.rs`** — `PitchRoll` state: two aligned ring buffers
  (line `samples` + `heat` columns, ~600 frames = visible span) + an **eased view
  window** that auto-frames on the played range without per-frame jumpiness (min
  span, padding, holds while silent). The line keeps a light octave-slip reject
  (median-based, interval-keyed — heat is the truth backstop). Both layers gated on
  input level so rests are clean (each bank column is normalized to its own max, so
  without the gate silence would show noise). Unit-tested (framing; buffer caps;
  slip rejected + trill kept; sustained leap accepted).
- Wired as `WorkspaceTab::PitchRoll` ("Pitch Roll"); opens from the **Panels** menu.
- **Live-verified** (render only) via the sway+grim recipe below with a temporary
  `seed_demo` (rising line → rest → fast D5↔E5 trill → hold, with synthetic heat
  columns): heat + line + gaps + trill-in-heat/smooth-line all render correctly;
  harmonics fall above the framed view (clipped → no octave spikes). Demo reverted.
  **Still needs a real-instrument pass** — dev env has no audio input.

---

## Phases

### Phase 1 — MVP live notation ✅ DONE
- [x] Treble-clef staff with correct note placement + ledger lines (both sides).
- [x] Live note from YIN detector drawn emphasised at the right edge.
- [x] Rolling written history: finished notes scroll left ("play and see what you
      wrote"). Newest is pinned at the right; older slide off toward the clef.
- [x] Intonation feedback: notehead colour green→red by cents, a `±cents` pill,
      and an in-tune needle bar. Past notes keep their as-played colour.
- [x] Note-capture state machine: glitch rejection (`MIN_NOTE_SECONDS`), dropout
      grace (`RELEASE_SECONDS`), cents EMA — unit-tested.
- [x] Respects global `AccidentalStyle` (sharps/flats) and concert pitch.

### Phase 1.1 — real clef, silence gate, waterfall ✅ DONE (2nd session)
Driven by the first live screenshot (ghost "D0" notes, plain vector clef):
- [x] **Silence/junk gate.** The engine never reports silence
      (`SILENCE_RMS_THRESHOLD == 0.0`), so the detector emitted low-freq noise that
      the panel drew as ghost notes (~37 Hz "D0" with giant ledger stacks). Fixed
      by gating capture on **input level + YIN clarity + a sane MIDI range**
      (`MIDI_MIN..=MIDI_MAX`), plus a defensive ledger-line clip to the panel rect.
- [x] **Real treble clef.** Embedded **Noto Music** (SIL OFL 1.1) at
      `assets/fonts/NotoMusic-Regular.ttf`, registered as the `"music"` egui font
      family in `theme::install_fonts`. Clef = glyph **U+1D11E**, accidentals =
      ♯/♭ glyphs. Alignment: the glyph eye sits ~0.235·fontsize above the G line
      (tuned on-screen; see verification note). Size ≈ `3.7 × gap`.
- [x] **Pitch "waterfall".** A live trail (`StaffTrainer.trail`) of the continuous
      detected pitch, drawn as a fading blue glow at its exact staff height right
      on the lines, behind the noteheads — shows glide/vibrato/latency and flows
      into the current note.
- [x] Latency: largely inherent to the analysis window + frequency smoothing in
      `audio::core` (shared by all panels, not changed). The level gate makes the
      note **clear promptly on silence** so it no longer feels "stuck".

**Follow-ups noted while building (backlog, low priority):**
- The intonation needle can read like a slider at a glance — consider ticks/label.
- History scroll is per-note (discrete), not smooth pixel scroll.
- Octave errors from YIN (e.g. sub-octave) can still write a wrong-octave note if
  in range; the waterfall reveals them. A median/октаve-lock could help (Phase 2).
- Accidental glyph vertical centring is close but could be nudged.

**Live-verification recipe (sway + grim), for future visual work:**
Temporarily (a) put `Staff` first in `default_workspace_tree` so it's the active
tab, (b) add a `STAFF_DEMO` env branch calling a `seed_demo` that fills
history/current/trail with synthetic data, then `rm ~/.local/share/fretboard/app.ron`,
launch with `STAFF_DEMO=1`, find the window via `swaymsg -t get_tree` (app_id
`fretboard`, `visible:true`) and `grim -g "<rect>"`. Revert (a)+(b) after. This is
how the clef offset was tuned without a live instrument.

### Phase 1.2 — fast resonator waterfall on the staff  ✅ DONE (3rd session)
Latency work. The staff wrote notes from the **YIN** reading only
(`AudioEngine::reading`), whose window (~6144 samples ≈ 139 ms) + `smooth_frequency`
EMA make the committed note lag noticeably. The **resonator bank** already runs
per-sample and publishes at ~60 Hz (`ResonatorSettings::update_ms = 16`) with its
Δφ instantaneous-frequency reassignment — a genuinely low-latency pitch-energy
view that the staff was ignoring.
- [x] **Draw `reading.resonator_waterfall` on the lines.** New
      `draw_resonator_waterfall` in `app::staff_panel`: columns = the bank's
      magnitude history (newest at the playhead, older left), each bin above
      `RES_WF_GATE` painted as a small ~gap-sized blue rect at *its own* pitch
      height via `ui::staff::midi_to_y` (same diatonic mapping as the noteheads).
      Bins outside the visible staff (the bank spans C0..C6, wider than violin)
      are clipped out, keeping it sparse + cheap.
- [x] Bin→pitch derived from row length ÷ semitone span (survives the reassign
      toggle changing bins/semitone). No new `StaffTrainer` state — reuses the
      history the engine already keeps.
- [x] Layered *below* the YIN blue trail and the noteheads, so it reads as fast
      "heat" flowing into the (slower, quantised, intonation-coloured) notes.

**Follow-ups / open visual choices (need a live instrument to judge):**
- Not yet verified on-screen with real audio (dev env has no instrument). Tune
  `RES_WF_GATE`, `cell_w/cell_h`, alpha, and colour live; the YIN trail may become
  redundant once the resonator heat is dialled in — consider dropping it.
- If shape count bites on a busy signal, mesh it like `ui::waterfall::draw_waterfall`.
- Could also use the resonator peak as a fast **onset/pitch prior** to shorten the
  YIN commit delay — see the algorithm survey + latency levers in
  [`pitch_detection_survey.md`](pitch_detection_survey.md).

### Phase 1.3 — resonator bank drives notes, YIN locks the octave  ✅ DONE
Latency endgame. The played-note source is the **fast resonator bank** (survey
recommendation #1) for pitch class + cents + onset timing, with **one** borrowed
signal from YIN — the octave. Started life pure single-lane (bank only); a live
bowed-string test showed constant **octave wandering** (the 2nd harmonic drifts
louder/quieter than the fundamental, so a per-frame harmonic score flip-flops
between f0 and 2·f0). Fix = the *minimal* fusion: keep the bank's fast fine pitch,
snap only its octave to YIN's octave-robust estimate when YIN is confident
(`YIN_OCTAVE_CONFIDENCE`). Not the rejected multi-rule fusion — a single octave-snap
line in `draw_staff_card`. YIN runs unconditionally so this adds no latency to
timing/cents; during sustain (where the wandering showed) YIN is rock-steady and
pins the octave.
- [x] **`dsp::analysis_math::resonator_fundamental`** — harmonic-aware fundamental
      from one reassigned resonator column. Scores every bin *as a fundamental* by
      summing its first `RESONATOR_HARMONICS` (fixed `+12·log2(h)` semitone offsets
      on the log-pitch grid), weighted `1/h`, and requires real energy at the bin
      itself. Beats a plain argmax on both **octave-up** (crowning an overtone) and
      **sub-octave** (half-pitch phantom). Sub-bin refined via `parabolic_tau`.
      Unit-tested (picks fundamental over a *louder* overtone; lone peak; silence).
- [x] **Carried on `TunerReading::fast_pitch`** `= Option<(fractional_midi,
      strength)>`, computed in `resonator_snapshot` (both reassign on/off) and
      stamped onto the reading by *both* publish paths via `SharedState::fast_pitch`
      (so the 40 ms YIN rebuild doesn't blank it). Published at the bank's ~16 ms
      cadence → ~100 ms ahead of the YIN commit.
- [x] **`staff_panel` reads `fast_pitch`** instead of deriving pitch from YIN:
      nearest semitone = note, fractional part = cents. Still gated on absolute
      input `level` (the normalised column always reports *some* fundamental, so
      `level` is the real silence gate) + `MIDI_MIN..=MIDI_MAX`. The
      `StaffTrainer` state machine, trail, and waterfall are unchanged.
- [x] **GOTCHA (fixed):** the resonator bank only runs while a consumer keeps
      calling `AudioEngine::request_resonator()` (a CPU-saving park gate in
      `native::mod`/`worker`). YIN runs unconditionally, so the *old* staff worked
      without asking; the new one must call `request_resonator()` every frame or
      `fast_pitch` stays `None` and the panel shows "play a note…" forever. Done in
      `draw_staff_card`, mirroring `resonator_panel`/`scale_finder`.

**Not yet verified with a live instrument** (dev env has none). When testing, watch:
- Does the written note now appear ~instantly vs. the old YIN lag?
- Octave stability on open strings / low positions (harmonic scoring should hold;
  if it slips, `pYIN`/HMM is survey step #2 — but only if this proves insufficient).
- Intonation cents: the reassigned bank grid is ~12.5 cents/bin + sub-bin refine +
  Δφ; if cents feel coarse vs. the old YIN, that's the one thing YIN did better —
  revisit whether cents (only) should come from YIN.
- Tune `FUNDAMENTAL_FLOOR` (resonator.rs) / `LEVEL_GATE` (staff_panel.rs) live.

### Phase 1.4 — probabilistic YIN (pYIN) octave anchor  ✅ DONE (built, not live-verified)
The octave-lock in 1.3 borrowed the *plain* YIN octave, which itself octave-errs on
hard frames. Replaced the whole pipeline pitch path with **pYIN** (Mauch & Dixon
2014, survey's top upgrade) — the principled cure for octave errors, not a threshold
tweak. New module `audio::dsp::pyin`, two stages:
- **Candidates** — instead of one YIN threshold, integrate YIN's period pick over a
  Beta(2,18) *distribution* of thresholds (100 samples). Each period the picks land
  on becomes a candidate weighted by the prior mass that chose it; the mass that
  found no dip is the frame's unvoiced probability. So each frame emits several
  weighted hypotheses (f0 and its octave alternatives), not one.
- **HMM** — an online Viterbi over 10-cent pitch bins + an unvoiced state.
  Transitions are **diagonal-dominant** (`SELF_STAY`) so holding a pitch is cheap
  (the key bug found in build: a kernel normalised over the whole ±octave window
  made "stay" cost ~ln(0.057)/frame and the unvoiced self-loop out-raced every
  voiced state — the tracker never committed). Octaves sit far in the Gaussian tail
  → a lone octave-outlier frame can't move the path; genuine leaps re-enter via the
  unvoiced state (uniform over pitch). Decoded greedily (argmax of the forward
  trellis) for zero added latency. Reports the winning *candidate's* frequency
  (sub-cent, so the tuner stays sharp) + voiced prob as clarity.

Integration is minimal: `PitchTracker` lives on `AnalysisPipeline` (stateful, per
stream); `analyze_window` takes the pitch as a param; `publish_analysis_reading`
drops the old `smooth_frequency` EMA (the HMM already smooths). Removed the now-dead
`detect_pitch_yin` + `smooth_frequency`/`correct_octave_jump`; kept `cmndf` as the
shared substrate. Everything downstream (staff octave-lock, tuner, scale finder)
just gets a better `reading.frequency_hz` for free. Unit-tested: candidate
extraction finds the tone, the HMM rejects a lone octave outlier but follows a
sustained octave change, silence → unvoiced, end-to-end tracks a clean tone.

**Live-verify checklist:** octave wandering gone during sustain? note onset settles
its octave within ~150–200 ms (window + a few HMM frames)? tuner cents still smooth
(should be — freq comes from the candidate, not the 10-cent bin)? If big *legato*
leaps (no bow gap) lag, that's the unvoiced-reentry routing — loosen `SELF_STAY` /
widen `TRANS_SIGMA_CENTS`.

### Phase 1.5 — fuse the resonator bank into the pYIN HMM  ❌ REVERTED by Phase 1.7
> **Superseded — kept for the record.** This phase moved the melody line off the fast
> bank and onto the fused `reading.frequency_hz`, and the fusion it traded for
> **contributes exactly nothing** (measured). It shipped "built, not live-verified"
> and cost the staff ~100 ms. See Phase 1.7 for the measurements and the revert.
> Its two live-verify questions below both had the answer "no".

Unify the two pitch sources into one tracker instead of the panel-side octave-lock
snap. The bank's fast pitch (`SharedState::fast_pitch`) is now fed into
`PitchTracker::process` as **one extra weighted HMM candidate** (`BANK_WEIGHT ×
strength`), gated on window level (`BANK_FUSE_LEVEL`, so the bank's normalised noise
floor doesn't leak in). Because the bank leads YIN's long window by ~100 ms, the
fused candidate (a) quickens the tracker's onset response and (b) lets bank + YIN
agree on the octave *inside* the HMM — a wrong-octave bank frame is out-voted by the
HMM's continuity (unit-tested: `process_rejects_wrong_octave_bank`,
`process_with_agreeing_bank_tracks`).
- The staff panel dropped its octave-lock: it now writes notes straight from the
  fused `reading.frequency_hz` (nearest semitone + cents), gated on `CLARITY_GATE`
  (pYIN voiced prob) + level. Simpler, and the octave logic lives in one place.
- Opportunistic: if the bank is parked (no `request_resonator` consumer), the HMM
  just runs YIN-only — same as before. The staff requests the bank, so it fuses.

**Live-verify:** onset feels quicker than pYIN-alone? octave still rock-steady
(bank shouldn't reintroduce wandering — the HMM gates it)? If the bank drags the
octave on hard notes, lower `BANK_WEIGHT`; if fusion feels no faster, raise it.

Next up (this session's roadmap, in order): **onsets/segmentation** → **start
latency** (onset-triggered HMM attack re-init + shorter/asymmetric window).

### Phase 1.6 — onset detection: segmentation + attack-mode latency  ✅ DONE (built, not live-verified)
Two of the roadmap's three directions, tightly coupled through one onset signal.
- **`audio::dsp::onset::OnsetDetector`** — energy-attack detector off the window
  RMS (~40 ms). Tracks a slowly-adapting baseline and fires on a rise above it; the
  key trick is the **re-arm** (must dip before firing again) so a sustained note
  gives *one* onset but a re-bowed repeat (dip→rise, no pitch change) fires again.
  Ratio-based → gain-robust. A monotonic `onset_seq` counter rides on
  `TunerReading` (a sequence, not a per-frame flag, so the UI can diff it safely
  across the 40 ms↔60 fps cadence gap).
- **Segmentation** — `StaffTrainer::update` takes `onset_seq`; a change forces a new
  note even at the *same* pitch, so repeated same-pitch notes split instead of
  merging (unit-tested `re_attack_splits_same_pitch`). Consumed only in the voiced
  branch, so an onset during the attack's unvoiced flicker stays pending until the
  pitch returns.
- **Attack-mode latency** — the same onset bool is passed into
  `PitchTracker::process`. On an onset frame the bank candidate is trusted heavily
  (`ATTACK_BANK_WEIGHT`) and the HMM trellis bias is dropped, so a new note — even a
  big leap — localizes to the fast bank at once instead of routing through the
  unvoiced state (~2–3 frames). Crucially this is gated on the onset: *off* an
  onset, continuity still rejects a wrong-octave bank glitch — the onset flag is
  what separates "new note" from "glitch" (unit-tested both ways:
  `onset_lets_bank_lead_a_leap` vs `process_rejects_wrong_octave_bank`).

**Live-verify:** repeated same notes now split into separate noteheads (bow changes
visible)? big interval leaps snap in faster? no spurious splits mid-sustain (if so,
raise `RISE_RATIO`/`REARM_RATIO` or `REFRACTORY_FRAMES` in `onset.rs`)? attack mode
doesn't cause wrong-octave flashes at note starts (if so, lower
`ATTACK_BANK_WEIGHT`)? The onset detector runs at the 40 ms analysis cadence — if
timing feels coarse, the next lever is a callback-rate energy envelope.

### Phase 1.7 — melody rides the bank again; latency is now measured  ✅ DONE
Reported live: "показ нот в violin staff стал очень тупить, не показывает текущее"
— and the pitch roll too. Both panels read `reading.frequency_hz`, so both were slow.

**What the measurements said** (all reproducible; the probes are now regression
tests, see below):

| path | note change | notes |
|---|---|---|
| pYIN on the 6144 window | **128 ms** | equals the window length *exactly*, at every window size (1024→21 ms, 8192→171 ms). The HMM will not leave a note while the window holds *any* trace of it. |
| pYIN, octave leap | **248 ms** | +120 ms of transition cost on top |
| plain YIN (pre-1.4 reference) | 128 ms | so the window, not the HMM, sets the floor for ordinary intervals |
| staff end-to-end, before | **128 ms / 328 ms** (8ve) | + `OctaveGate` + the 140 ms release grace |
| **resonator bank** | **8–29 ms** | inside the ~30–40 ms perceptual threshold |

**Why Phase 1.5's fusion could never work.** The bank was fed to the HMM as one
weighted candidate at `BANK_WEIGHT × strength ≤ 0.5`, but YIN's own candidate
measures `p = 1.000` — the bank loses every frame at *any* signal strength, and an
octave transition cost of ~18 nats buries it besides. Feeding the HMM a bank reading
10 ms ahead of the window changes the output **by exactly zero**. Raising
`BANK_WEIGHT` (what 1.5's live-verify note suggested) would not have fixed it either:
the real blocker is that YIN's *emission* for the old note stays at `p = 1.0` until
the window flushes, and no candidate weight touches that.

**The revert, done once for both panels.** New `audio::dsp::melody::MelodyTracker`
restores 1.3's architecture — bank leads, pYIN pins the octave — but as a shared
value rather than a line in `draw_staff_card`: it rides `TunerReading::melody_pitch`,
computed in `publish_resonator_snapshot` at the bank's ~16 ms cadence and re-stamped
by the 40 ms path. `frequency_hz` stays pYIN's, for the tuner and fretboard, where a
steady reading beats a prompt one.

Two guards the naive 1.3 snap did not have, both found by the tests, not by reasoning:
- **Stale anchor, different note** — for ~128 ms after a leap the anchor is still on
  the *previous* note. A naive snap of a fresh E5 toward a stale A4 computes
  `round((69−76)/12) = −1` and lands on **E4**. Fixed by requiring pitch-class
  agreement (`OCTAVE_AGREE_SEMITONES`).
- **Octave leap vs octave slip** — the one case pitch class provably cannot separate
  (A4 and A5 are the same class). Trusting the anchor here measured **261 ms**.
  Separated by *time* instead (`LEAP_CONFIRM_FRAMES`): wandering's disagreement is
  intermittent and resets the count, a real leap's is unbroken.

**Also fixed:** `LEAP_MASS` in the HMM kernel (a fifth sat 10σ out in the Gaussian,
so only the 1e-6 floor kept its cost finite; legato leaps have no unvoiced frame to
route through, which the design assumed they did) — octave 248→208 ms. `OCTAVE_REJECT`
7→11 semitones (7 rejected a perfect *fifth* — every violin open-string crossing; it
only passed at all because the tracker reports 75.95 rather than 76.0).

**Result:** 128→**28 ms** for every interval that changes pitch class, 328→**111 ms**
for an octave leap.

**The real lesson — why this shipped at all.** 1.4/1.5/1.6 were each marked
"✅ DONE (built, not live-verified)", and *not one* of them had a test that measured
latency. Every pYIN test fed the **same window** over and over, which cannot observe
a note change by construction. The probes are now regression tests with budgets:
`resonator::bank_latency_probe` (≤40 ms), `staff_panel::end_to_end_latency_probe`
(≤60 ms, ≤130 ms for the octave), plus `pyin::latency_probe` /
`short_window_accuracy_probe` as reference numbers. A design that trades latency for
tidiness must now fail a test, not a live session weeks later.

**Still open (measured, not fixed here):**
- **Nothing above ~1000 Hz.** `cmndf`'s `min_lag = sr/1000` caps YIN at **B5**, and
  the bank's own `ResonatorSettings::max_midi = 84` caps it at **C6**. A violin plays
  well above both in high positions. Raising the bank's ceiling costs resonators
  (CPU) — a settings decision, not a bug fix.
- `LOWEST_TRACKED_FREQUENCY = 16 Hz` makes `cmndf` search lags to 3000 samples
  (13.9M ops/frame) for a band no instrument here plays; the low tail is also where
  the sub-bass ghost lives.
- The now-inert `BANK_WEIGHT`/`ATTACK_BANK_WEIGHT` fusion still runs in `pyin`. It is
  harmless but it is dead weight, and it is what made 1.5 look reasonable.

### Phase 1.8 — universal ceiling, quantile framing, one octave decision  ⚠ BUILT, NOT LIVE-VERIFIED
Landed `969e37b` + `a063606`, straight after 1.7 was confirmed by ear. Labelled
honestly: 1.7's fix is verified, **this is not** — see the handoff at the top.

**The ceiling was silently transposing, not dropping.** `cmndf`'s `min_lag = sr/1000`
capped YIN at B5, and `yin_pick` scans lags upward taking the first dip — so with the
true period excluded, the first dip still *reachable* is the note's sub-octave.
Measured C6/E6/A6 at exactly **−12.00 semitones**: the upper half of a violin's range,
an octave flat, on the tuner and fretboard too. Made the bounds universal rather than
accidental — `HIGHEST_TRACKED_FREQUENCY` = C8 (4186 Hz) beside the existing C0 floor,
pyin's grid to match, and `NOTE_BUCKET_MAX_MIDI` 84→108 so the bank (which *is* the
melody's pitch) can hear up there at all. C0..E7 now tracks within 0.04 semitones,
asserted by `ceiling_probe` — a ceiling fails silently, so it must fail a test.
Cost: bank 360 → 480 resonators.

**Pitch roll framed on the whole waterfall.** `reframe()` took min/max over all 600
frames (~10–20 s), so a phrase from fifteen seconds ago kept the view stretched and one
slip pinned it an octave wider. At three octaves the rows fall under
`pianoroll::LABEL_MIN_ROW_H` and only the Cs stay labelled — reported as "больше не
рисуются ноты, только октавы", which was never a labelling bug. Rebuilt on three rules:
recent frames only (`FRAMING_FRAMES`), quantile bounds (`VIEW_QUANTILE_*`) instead of
min/max, hysteresis on shrink (`VIEW_SHRINK_SLACK`). Growing is no longer eased at all —
at `VIEW_EASE` a leap took ~0.6 s to cover and the line was *off screen* for it.

**The octave is now decided once, off the UI clock.** It had been judged in four
places, the last being `OctaveGate` — living in the panels, duplicated in both, driven
**per UI frame**: a DSP filter whose median window was measured in frames, so a
stuttering UI quietly changed its timescale. Moved into `dsp::melody` at the bank's
16 ms cadence, which now states its three layers plainly (bank harmonic scoring → snap
to pYIN's octave → gate for what's left). The move surfaced a real interaction: the
gate rejected the very leap the hysteresis had just confirmed, its median still on the
old octave — a second conservatism tax on a decision already paid for over five frames.
A confirmed leap now resets the gate: **octave leap 111 → 78 ms**.

Panels now do **no DSP at all**. Writing `note_detection.md` immediately exposed the
last of it: staff clamped to 12..103, pitch roll to 24..103 "matching the pYIN
tracker's own grid" — untrue since the melody moved to the bank (12..108), with the
staff's G7 ceiling sitting *below* the grid's C8. Two private second opinions about the
range, both already drifted. Removed; the melody's range simply is the bank's.

### Phase 1.9 — the decision engine comes off the UI clock  ⚠ BUILT, NOT LIVE-VERIFIED
The last place a dropped frame could change a musical decision. Note segmentation
(`MIN_NOTE_SECONDS`, `RELEASE_SECONDS`, `CENTS_EMA`) ran inside `StaffTrainer`, driven
by `ui.input(|i| i.time)` — so a stutter could commit a note early or fail to, and every
note's *duration* was measured in renderer ticks. It is now
**`audio::dsp::segmenter::NoteSegmenter`**, driven at the bank's cadence with `now`
taken from the **sample count**, and the panel reads the finished line off
`TunerReading::note_line`. Same rule, same reason as the `OctaveGate`'s move in 1.8
(`note_detection.md` §4); this was the other half of it.

**The silence gate had to move with it** — a decision needs to know when the sound
stopped, and the gate was sitting in the panels (duplicated verbatim in both, same
`0.02`). It now gates the bank's *input* in `publish_resonator_snapshot`. That fixed
something the panels were hiding: `MelodyTracker` was fed raw bank pitch regardless of
level, so its leap/slip hysteresis stayed alive on room noise through every rest — the
panels only gated the result. Silence now properly ends the phrase.

**Falls out of the move:** `audio::dsp` is private again in every build. It was
`pub(crate)` under `cfg(test)` purely so the end-to-end probe could live in
`app::staff_panel` and reach across; the whole path it measures is now inside `dsp`, so
the probe sits next to it and the hole in the wall is gone.

**Latency is unchanged** — 24/40/72 ms, but measured to the *engine's decision* rather
than to the UI frame that draws it (add ≤17 ms for the pixels). Not a speed-up; the
ruler ends in a different place. See `note_detection.md` §6.

**The tests that were missing, and what they found.** Every DSP test either drove one
module or composed them by hand — none touched the *wiring*, which is exactly what this
change rearranged (a level atomic one plane writes and the other reads, an onset counter
crossing between them, a sample clock, `note_line` stamped by both publish paths). A
wiring mistake there is invisible to the whole DSP suite and total to the user: the staff
just stays empty. So `core::engine_writes_a_played_note_end_to_end` now drives both real
pipelines through the real `SharedState`.

It immediately found a **real, pre-existing bug** (see below). That is the 1.7 lesson
landing again: the suite could not see it because nothing tested the thing being claimed.

**🐞 Release ghosts — found, measured, NOT fixed.** After a note stops, the staff writes
notes nobody played (measured: A4 → a ghost at MIDI 13, a C#0 with a ledger stack). Two
causes line up, both upstream of segmentation:
1. **The silence gate closes ~500 ms late.** `input_level` is the *smoothed* level
   (×0.88 per 40 ms frame falling) — smoothing that exists so the UI's level *meter*
   doesn't flicker, inherited by the gate through the shared atomic.
2. **The bank cannot be quiet.** Its column is normalized to its own max, so once the
   note stops it reports whichever bins ring longest (the low ones decay slowest) at
   full confidence. `OctaveGate` rejects ~50 ms of it, then its median moves onto the
   garbage and passes it.

Pinned by `core::release_ghosts_are_written_after_a_note`. **Not fixed here on purpose**:
it is a DSP behaviour change that needs its own live verification, and bundling it into a
refactor whose whole claim is "nothing changed but the clock" would make both
unverifiable. **How bad it is live is unknown** — the test cuts the tone off instantly,
which no instrument does; a real note decays over hundreds of ms and the gate and the
bank may fall together. Likely fix: give the melody gate the *raw* window level instead
of the meter's smoothed one. **Ask the instrument first.**

### Phase 1.10 — the panels' history comes off the UI clock  ⚠ BUILT, NOT LIVE-VERIFIED
Landed `518457e` (pitch roll) + `055e9e0` (staff). §4's other half — the one 1.9 left,
and the one filed as *display fidelity*. It was not.

**A panel sampling `melody_pitch` per repaint makes the renderer the sampler.** The
bank publishes at ~62 Hz, so a 60 fps panel dropped a few percent of its frames and a
30 fps one dropped **half** — decimating the trills and vibrato the fast path exists
for. `pianoroll`'s own module doc claimed "trills and vibrato appear at full
resolution"; it was false at every frame rate.

**`seq × cadence` would have been a third wrong ruler.** The plan said "drains by
sequence", and counting seq×16 ms is the obvious next step — but the publish is gated
on the *wall* clock and fires on the first sample batch after the interval expires, so
frames land ~16 ms apart **with jitter**. Each frame carries `t` off the sample count
instead: the ruler 1.9 already built for note durations.

**The ~70 MB/s the plan feared is a cursor away.** That is the cost of copying the
whole history per read; a delta is 1–2 columns ≈ 0.23 MB/s. `melody_since(after)` is a
cursor and not a drain, so the staff and the roll read the same frames rather than
stealing them. On wasm the worker posts the delta and the main thread rebuilds the ring
(no shared memory) — same API both platforms.

**The staff had the parallax the plan didn't know about.** Reported live mid-session:
"два водопада… один медленнее другой быстрее". Both layers drew the same sound over the
same x range in different units — the trail in 240 *UI frames* (~200 px/s), the heat in
52 *bank columns* squeezed by a `clamp(2.0, 6.0)` into ~312 px (~375 px/s). The plan had
the trail down as "the same shape and the same excuse", one layer; it was both, and the
second was worse. They are two views of one `MelodyFrame` now, through one `x_of`.

**The ruler is drawn** (−2s / −4s / …) on the roll: the axis was unverifiable by eye,
which is how it stayed wrong.

**What the tests found.** `history_is_paced_by_the_audio_not_the_frame_rate` feeds a
trill and asserts a panel repainting half as often holds every alternation.
`core::the_melody_history_is_published_on_the_audio_clock` drives the real pipelines. And
`a_restarted_clock_clears_the_history` caught the panel **claiming the self-healing rule
in a comment while not implementing it** — which is what pushed the ring into one shared
`MelodyHistory` instead of two hand-rolled `VecDeque`s.

**Left alone, deliberately:** the staff's noteheads still step `gap * 3.2` per *note*,
so a written note and the heat that produced it drift apart — a third ruler, but a real
design question (notation is not a time plot), not a mistake. Answered in 1.11.

### Phase 1.11 — the staff draw split, and the second ruler  ✅ DONE (`9985bce`)
Landed as a **pure refactor**: seven regions in one 8-argument function became one
function each, called in the order the ink lands (`staff_geom` → `draw_engraving` →
`Waterfall::draw` → `draw_noteheads` → `draw_note_names` → the intonation bar), with
`draw_staff` as the running order painting nothing itself. Two invariants became types:
`TimeRuler` (the waterfall's **one** ruler — the thing 1.10's parallax was) and
`Waterfall` (the frame context its two layers draw from, so neither can invent its own
span). `BankRange` retires the swappable `res_min_midi`/`res_max_midi` pair.

**DESIGN DECISION (user, 2026-07-15) — the staff keeps two rulers, on purpose.**
The waterfall is placed by TIME; the noteheads by COUNT, one column per note. So a
written note and the heat that produced it sit at different x and drift further apart
the longer the note is held. **This is intended and is not to be "fixed".** Notation is
a reading surface, not a time plot — a whole and an eighth take one width on paper.
Rejected alternatives: placing the heads at `x_of(t)` (that is a piano roll, which the
pitch-roll panel already is), and splitting the two into visually separate bands. The
two rulers agree only at the playhead. This lives in `note_columns`' doc comment and is
pinned by `the_written_line_is_placed_by_count_not_by_time` — the placement rule used to
be buried inside a painting loop where nothing could check it.

Behaviour holds by design: 1.8–1.10 are still unverified on the instrument, and a
refactor that also moved pixels would make unverified indistinguishable from broken.
Two knowing cosmetic deltas are listed in the commit message (cell width reads
`px_per_second` directly; note names are drawn after all the heads, so the header row
stays readable where a high head reaches into it). Tests 112 → 116.

### Phase 1.11 — SWIPE′ salience: the octave decision by construction  ⬜ DESIGNED, NOT BUILT
Design: [`swipe_salience_design.md`](swipe_salience_design.md). Triggered by a live
report — **on the G string, bow strokes throw the pitch to the phantom octave**.

**The literature describes our bug literally, and on our frequencies.** Camacho's SWIPE
thesis enumerates three scorings (Fig. 3-13): positive lobes only → *"peaks at sub and
supraharmonics"*; add negative valleys → the supraharmonic peak *"has disappeared"*; add
first-and-prime harmonics → *"a major peak only at the fundamental"*.
`analysis_math::resonator_fundamental` is **panel C** — a reward-only comb
(`score += column[b+offset(h)]/h`) that cannot subtract, so it has no way to punish 2·f0.
The phantom octave is the supraharmonic peak, by construction.

**And SWIPE was designed on our exact signal.** Its worked example for choosing the
spectral warping is a spectrum with *"a missing fundamental and a salient second
harmonic"* — fundamental at **190 Hz**, 2nd harmonic at **380 Hz**. The violin G is
196/392: the instrument barely radiates its own open-G fundamental. For candidate G4=392,
G3's *odd* harmonics (588, 980) land exactly in the kernel's negative valleys and cancel
it; for candidate G3=196 every harmonic lands in a positive lobe **even when the 196 bin
is empty**. Score comes from the harmonics, not the fundamental's own bin.

**`FUNDAMENTAL_FLOOR = 0.18` is a crutch for the missing valleys, and it is what kills the
G string.** Its comment says its job is blocking the sub-octave — which a reward-only comb
needs, and which primes do properly. It is a hard cliff sitting exactly where a weak
fundamental lives.

**Second root cause — why nothing catches it, and it is the "просто от bow strokes" in the
report.** `OnsetDetector` fires on every bow stroke *by design* (its doc: a re-articulated
note "dips below the baseline and re-arms, so its second attack fires too" — the staff
needs that). But `pyin::process` on an onset frame weights the bank at
`ATTACK_BANK_WEIGHT = 2.0` **and drops the trellis** (`initialized = false`), so the frame
is decided by emissions alone and the bank beats YIN's `p = 1.000`. **On a bow stroke pYIN
outputs the bank's pitch.** The anchor then *agrees* with the bank's phantom octave, so
`snap_to_anchor_octave` snaps to it, `octave_dispute` never counts, and `OctaveGate`'s
median follows. Every layer built to catch the bank's octave error is fed by that error on
exactly the frames where it fires.

The docs say the opposite, confidently: `melody.rs` "provably contributes nothing"; this
plan "harmless", "inert", "dead weight". That was measured for `BANK_WEIGHT` with the
trellis **intact**; `ATTACK_BANK_WEIGHT` with the trellis **dropped** is a different
regime and the measurement does not transfer. Phases 1.4–1.6's own trap in miniature — a
number verified in one condition, restated as a property. **Kill the fusion first**: it is
independent of the salience work, cheap, and an anchor that echoes the bank is not
evidence no matter how good the bank gets.

**Found on the way: a display slider steers the octave decision.** `normalize_bars(…,
settings.gamma)` at `resonator.rs:337` feeds `resonator_fundamental` at `:338`, and
`gamma` is the waterfall-contrast slider (`controls.rs:763`, `0.15..=2.4`). Same disease
as 1.9's silence gate sharing the UI meter's smoothing: a UI filter steering DSP. Third
instance of "a number verified once, then trusted forever" in this phase alone.

**Why it fits here rather than fighting the app.** On a log-frequency grid the SWIPE′
kernel depends only on `f'/f`, so it is **one fixed vector** and the whole salience curve
is a single cross-correlation with the √-warped column. The harmonic set falls out of the
grid resolution (`{1} ∪ primes ≤ 13` at 8 bins/semitone) instead of being picked. And the
bank already is the per-candidate filter SWIPE fakes with pitch-dependent windows.

**What it deletes** (by construction, not tuning): `FUNDAMENTAL_FLOOR`,
`RESONATOR_HARMONICS`, the `1/h` weighting (the thesis tested `p ∈ {1/2,1,2}` and rejected
our `p=1`), the detector's use of `gamma`/`power` — and then, pending verification, the
whole repair layer above it: `snap_to_anchor_octave`, `YIN_OCTAVE_CONFIDENCE`,
`OCTAVE_AGREE_SEMITONES`, `LEAP_CONFIRM_FRAMES`, `OctaveGate`. ~7 hand-picked constants,
and the melody path's dependency on the slow pYIN anchor. The thesis's own claim:
*"there are no free parameters in SWIPE and SWIPE′, at least in terms of 'magic numbers'"*,
and SWIPE′ *"outperform[ed] all the algorithms on all the databases"* (12 competitors,
speech + musical instruments).

**This one does not have to join 1.8–1.10 in the unverified pile.** The defect is
offline-reproducible: a synthetic violin-G column (`[0.08, 1.0, 0.7, 0.5, …]`) must decide
G3, and today's code cannot pass — at 0.08 the fundamental never even becomes a candidate.
Sweeping the fundamental to *exactly zero* and asserting the decision holds is the "by
construction" claim made falsifiable. Full checklist in the design, §7.

**Biggest risk, decide before cutting** (design §4.3, §8): the reassigned column is spiky
where SWIPE expects a dense √-spectrum. The valleys should still collect the odd
harmonics — that is where the reassigned spikes are — but measure it on a real column
before building the rest.

### Phase 2 — Target / call-and-response mode  ⬜ NOT STARTED
Show a **target** (a single note, then a short phrase/scale) the user must play;
score hit/miss on pitch and grade the intonation. This turns the mirror into a
drill.
- Target note(s) drawn in a distinct "ghost" style ahead of the playhead.
- Advance when the played pitch matches within a cents tolerance held for N ms.
- Per-note verdict (correct/too sharp/too flat/wrong note) + a session score.
- Source of targets: reuse `core_types::scale` / `tuning` + the Scale Finder to
  generate exercises in a chosen key/scale.

### Phase 3 — Rhythm & timing  ⬜ NOT STARTED
Move from pitch-only to time-aware notation.
- Note **durations** from held time → quarter/eighth/half, with a tempo/metronome
  (reuse the Drone panel's BPM). Draw stems/flags/beams and rests.
- A moving **playhead**; optionally a fixed-tempo grid to practice *in time*.
- This is the big one — needs a quantiser and a real notation layout pass.

### Phase 4 — Sessions, save & export  ⬜ NOT STARTED
- Persist practice sessions; review past runs.
- Export the played line to **MIDI** and/or **MusicXML** (the "запись в файл"
  option deferred from Phase 1). MusicXML lets the line open in MuseScore etc.
- Import a MusicXML/MIDI exercise to use as a Phase 2 target.

### Phase 5 — Real engraving  ⬜ NOT STARTED
- Load a SMuFL music font (e.g. Bravura) via egui `FontDefinitions` and draw the
  clef, accidentals (♯ ♭ ♮), noteheads, rests as glyphs. Big visual upgrade;
  needs font-loading plumbing (currently the app uses egui defaults only).
- Key signatures; alto/other clefs if we generalise beyond violin.

### Phase 6 — Violin-specific coaching  ⬜ NOT STARTED
- String + finger/position hints per note (which string, 1st/3rd position…),
  tying into the existing Fretboard panel model.
- Double-stops / drones against the played note (reuse Drone) for interval
  training.

### Phase 7 — Intonation analytics  ⬜ NOT STARTED
- Aggregate cents error over a session; surface "problem notes"/positions.
- Track improvement over time (needs Phase 4 persistence).

---

## Testing / verification notes
- Geometry is unit-tested; run the visual preview with:
  `cargo test -p fretboard render_staff_preview -- --ignored --nocapture`
  then convert the PPM it writes (scratchpad) to PNG (`magick in.ppm out.png`).
  Override output dir with `STAFF_PREVIEW_DIR`.
- Note segmentation is unit-tested in `audio::dsp::segmenter::tests`; the engine's
  *wiring* (both pipelines through the real `SharedState`) in `audio::core::tests` —
  see `note_detection.md` §6 for why the second kind had to exist.
- Full live check needs real audio input; open the **Violin Staff** panel and
  play/sing — Phase 1 was verified by build + tests + offscreen render + a
  launch smoke test (no live instrument in the dev env).
