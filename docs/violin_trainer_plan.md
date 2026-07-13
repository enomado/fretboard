# Violin Trainer — multi-session plan

A living roadmap for the notation-based violin trainer. Update this file as phases
land; it is the hand-off between sessions. Keep phase status honest (done / in
progress / not started) and note the commit when a phase ships.

## Vision

A practice panel that renders **standard notation** (treble clef staff) and, while
you play the violin, **writes what you play** onto the staff in real time — with
**intonation feedback** so you can see how in-tune each note was. It grows from a
passive "mirror" (see what you played) into an active trainer (play *this*, get
scored).

Pitch source is the app's monophonic YIN detector (`AudioEngine::reading` →
`TunerReading { frequency_hz, cents, clarity }`), which is the right tool for a
single-line instrument and already gives cents for intonation.

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

### Phase 1.5 — fuse the resonator bank into the pYIN HMM  ✅ DONE (built, not live-verified)
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
