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

## ▶ Start here — handoff (2026-07-14)

### Where it stands

The melody line's latency regression is **fixed and confirmed by ear**: 128→28 ms for
ordinary intervals, 328→78 ms for an octave leap. Read
[`note_detection.md`](note_detection.md) first — it is the whole mechanism in one
place, and the rest of this section assumes it.

Three commits landed, in order:

| commit | what | live-verified? |
|---|---|---|
| `1a8c6e1` | melody rides the resonator bank again; latency measured | ✅ **yes** — "работает неплохо" |
| `969e37b` | universal ceiling (C8), quantile framing, one octave decision | ❌ **no** |
| `a063606` | systematisation doc; panel range clamps dropped | ❌ **no** |

**That table is the most important thing on this page.** The user's "работает неплохо"
was given *after `1a8c6e1` and before the other two*. Everything since is
tests-and-reasoning only — which is the exact state Phases 1.4–1.6 were in when they
shipped a 100 ms regression that took weeks to notice. Do **not** record `969e37b` /
`a063606` as verified on the strength of that sentence.

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

1. **Finish taking the decision engine off the UI clock** (agreed, not started — see
   [`note_detection.md`](note_detection.md) §4). Note segmentation
   (`MIN_NOTE_SECONDS`, `RELEASE_SECONDS`, `CENTS_EMA`) still runs inside
   `StaffTrainer` driven by `ui.input(|i| i.time)`, and the pitch roll's history is
   sampled per UI frame — its own comment admits the span is "~10 s at 60 fps, ~20 s
   at 30 fps", i.e. the waterfall's time axis is not time. Shape: a `NoteSegmenter` in
   `dsp`, clocked off the **sample count** (which also makes note durations
   sample-accurate instead of frame-quantised), note history on `SharedState`, panels
   left with clef/key/drawing only. This is the last place a dropped frame changes a
   musical decision.
2. **`LOWEST_TRACKED_FREQUENCY = 16 Hz`** makes `cmndf` search lags to 3000 samples —
   **13.9M ops/frame** for a band no instrument here plays, and the low tail is where
   the sub-bass ghost lives. Cheap win; needs a decision on the real floor.
3. **The inert `BANK_WEIGHT`/`ATTACK_BANK_WEIGHT` fusion** still runs in `pyin`. It is
   harmless and marked "do not fix latency with this", but it is dead weight and it is
   what made Phase 1.5 look reasonable in the first place.

### Landmines

- **The bank is park-gated.** Any panel reading `melody_pitch` must call
  `AudioEngine::request_resonator()` every frame or it sits at "play a note…" forever.
  YIN runs unconditionally; the bank does not.
- **`audio::dsp` is `pub(crate)` only under `cfg(test)`** (see `audio/mod.rs`). That is
  deliberate — the latency tests drive the real bank + tracker end to end, and the
  absence of exactly that test is how this regression shipped. Production code reads
  finished values off `TunerReading`.
- **`MelodyTracker::update` must be driven at the bank's cadence, once per bank
  frame.** Its hysteresis and the gate's median are counted in those frames. The 40 ms
  pYIN path deliberately re-stamps the last value instead of recomputing.
- Uncommitted in the tree and **not mine**: `Cargo.lock`, `src/ui/segmented.rs`.

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
- **`src/app/staff_panel.rs`** — `StaffTrainer` state (rolling note history + the
  note currently held, with a glitch/hold/release state machine) and
  `App::draw_staff_card`. Held on `App.staff`; not persisted.
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
- Capture state machine is unit-tested in `app::staff_panel::tests`.
- Full live check needs real audio input; open the **Violin Staff** panel and
  play/sing — Phase 1 was verified by build + tests + offscreen render + a
  launch smoke test (no live instrument in the dev env).
