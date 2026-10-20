# Pitch benchmark — the number the detector is argued against

`audio::dsp::pitch_bench` measures **Raw Pitch Accuracy (RPA)** of the pitch frontends
and the shipped melody line against a corpus with a perfect f0 annotation. This file is the operator's manual;
the *why* lives in the module's own doc comment and is summarised below.

Companions: [`note_detection.md`](note_detection.md) (what the pipeline is),
[`pitch_detection_survey.md`](pitch_detection_survey.md) (what the alternatives are),
[`../testdata/README.md`](../testdata/README.md) (the five real violin takes).

## Why a corpus benchmark exists at all

We already have `testdata/` — five takes of a real violin, recorded under a protocol.
They answer **"does the phantom octave still win?"**: a histogram over one note, on the
instrument the app is for. They cannot answer **"how good is this detector?"** — for that
you need many instruments, many registers, and a ground truth that does not depend on
somebody remembering what they played.

The trigger was Table 1 of Marttila & Reiss, ISMIR 2025 ([arXiv:2507.11233](https://arxiv.org/abs/2507.11233)):

| SWIPE, as reported in the self-supervised literature | 86.6 % |
|---|---|
| **SWIPE, their careful implementation** | **96.2 %** |

Ten points of RPA between two implementations *of the same algorithm*, from nothing but
search range and frequency-sampling scale. Our SWIPE′ sits somewhere on that spread and
we do not know where. Every downstream decision — Viterbi, a neural reweighting, a
different frontend — is unfalsifiable until that number exists, because the effects being
chased (the paper's Toeplitz layer buys **+0.4 points** on clean audio) are an order of
magnitude smaller than the uncertainty about our own starting point.

## The corpus

**MDB-stem-synth** — [Zenodo 1481172](https://zenodo.org/records/1481172), 230 solo stems
from MedleyDB, 25 instruments, 15.56 hours. Each stem is **re-synthesised from its own f0
annotation**, which is what makes the annotation *perfect* rather than merely careful —
there is no estimator in the ground-truth path to inherit errors from.

It is the corpus the paper reports on (their Table 3), so our number lands on their axis.

```sh
mkdir -p datasets && cd datasets
curl -L -o MDB-stem-synth.tar.gz \
  "https://zenodo.org/records/1481172/files/MDB-stem-synth.tar.gz?download=1"
tar xzf MDB-stem-synth.tar.gz
```

~1.85 GB compressed, ~5 GB unpacked. `datasets/` is git-ignored — this is third-party
audio fetched on demand, never committed.

Layout: `audio_stems/<track>.RESYN.wav` ↔ `annotation_stems/<track>.RESYN.csv`, the CSV
being `time,frequency` with **frequency 0 meaning unvoiced**.

## Running it

```sh
# the whole corpus — ~3.5 h of CPU (per-sample IIR bank over 15.5 h of audio)
cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# scoped: a quick loop (~1 min/track)
PITCH_BENCH_LIMIT=5 cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# scoped by track id substring (note: ids are *pieces*, not instruments)
PITCH_BENCH_FILTER=NightOwl cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# custom lag sweep; 0 must stay first
PITCH_BENCH_LAGS_MS=0,20,24,28 cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# by instrument (MedleyDB's name for the stem; build the table once)
uv run tools/medleydb_instruments.py
PITCH_BENCH_INSTRUMENT=violin cargo test --release --lib swipe_rpa_on_corpus -- --ignored --nocapture

# the other detectors, same harness
cargo test --release --lib sieve_rpa_on_corpus -- --ignored --nocapture   # docs/harmonic_sieve.md
cargo test --release --lib pyin_rpa_on_corpus -- --ignored --nocapture    # pYIN at its own 40 ms
cargo test --release --lib peak_comb_rpa_on_corpus -- --ignored --nocapture  # docs/peak_comb.md
```

Tracks run in parallel — `PITCH_BENCH_JOBS` threads, half the machine by default — and are
merged and printed in track order, so a parallel log is line-for-line the sequential one
(checked: identical on the full corpus). ⚠ `swipe_rpa_on_corpus` is a *substring* of
`rtswipe_rpa_on_corpus`; add `--exact` with the full path to run only the bank.

`--release` is not optional in practice: the bank is a per-sample IIR over ~670 resonators,
and a debug build turns an overnight run into a weekend one. Budget ~5 GB of disk for the
corpus **on top of** `target/`, which for this crate runs to tens of GB — the first run of
this benchmark died on `No space left on device`.

## The lag sweep — the thing this benchmark found

Every frame is scored against the annotation at several lags in **one** pass (running the
bank is ~99 % of the cost; a binary search into the annotation is free). The first run
came back like this **on two tracks** — kept because the whole sweep exists because of it,
and because the corpus went on to move its peak:

| lag (ms) | RPA | ±1-semitone share of misses |
|---|---|---|
| 0 | 83.81 % | 51.8 % |
| 8 | 88.42 % | 44.0 % |
| 16 | 90.97 % | 40.1 % |
| **24** | **91.86 %** | 43.7 % |
| 32 | 91.03 % | 50.5 % |

### The corpus, all 230 tracks (2026-07-15) — these are the numbers

The first full run: 4.7 h, 1.57 M voiced frames, `datasets/bench_runs/`. It is the baseline
any detector change is argued against, and **the two-track probe above was not it** — the
peak moved from 24 ms to **16 ms**, and every figure is a point or two lower.

| lag (ms) | RPA, full band | RPA, 65–2093 Hz |
|---|---|---|
| 0 | 82.99 % | 83.93 % |
| 8 | 87.93 % | 89.07 % |
| **16** | **90.18 %** | **91.02 %** |
| 24 | 89.84 % | 90.11 % |
| 32 | 87.64 % | 87.44 % |

Two tracks were never going to settle a peak — they were 0.9 % of the corpus, and the
sweep's spacing is 8 ms. Quote the corpus; the probe is history, not a second opinion.

Two things fell out of the original table, and only one survived:

1. **Half of our "errors" were timing, not scoring.** ±1-semitone misses were 52 % of all
   errors and near-symmetric (+1: 664, −1: 587) — the signature of a detector chasing a
   moving pitch, not of a kernel confusing harmonics (which would lean one way).
2. ~~**The benchmark independently measured the bank's group delay.**~~ The curve peaked at
   24 ms and fell off both sides, landing inside the 8–29 ms measured by an entirely
   different method (`resonator::bank_latency_probe`). **This reading is wrong — see
   below.** It is struck through rather than deleted because the coincidence was the whole
   reason to believe it, and the next person to find a peak inside an expected range
   deserves to know that this one was not the confirmation it looked like. The corpus then
   put the peak at 16 ms, which is *also* inside 8–29: a 21 ms-wide range accepts almost
   anything, which is the point.

So there are **two honest numbers**, answering different questions:

- **lag 0** — the *pipeline*, latency included. What the player actually gets.
- **lag ≈ 16 ms** (the corpus's peak) — the *scorer*. The number comparable to the
  literature, whose FFT-based estimators centre a symmetric window and are therefore
  delay-compensated by construction. Scoring our IIR at lag 0 against their centred FFT
  flatters *them*.

Neither is the "real" one; quoting only the flattering one would be the lie.

### A lag-sweep peak is not a group delay (2026-07-15)

The SwiftF0 oracle broke claim 2. SwiftF0 peaks at **+9.4 ms** on this corpus — and
SwiftF0 has no group delay to have: it is a centred STFT plus a conv stack, and the
reference resamples its frames onto our grid zero-phase. A peak the *delay-free* estimator
also shows cannot be a measurement of anybody's delay.

`tools/lag_sweep_probe.py` (one command, ~4 min) chases it down. Two attractive answers are
wrong, and both are worth knowing about because each survived a plausible-sounding argument:

- **"SwiftF0 is late."** It is not. Its `CENTER_OFFSET = 127.5` samples is a claim the
  Python cannot enforce — the pad is computed *inside* the ONNX graph — but reading the
  graph out shows a true 384/384 split, so the claim holds. And a vibrato-phase probe
  through the oracle's own path measures the model at **+1.4 ms**, with self-checks that
  recover an imposed 8 ms shift to within 0.01 ms.
- **"The corpus is offset."** It is not. A centred-window YIN at 44.1 kHz — no model, no
  resampling, zero delay by construction — matches MDB-stem-synth's annotation to
  **~1 cent at ~0 ms**. (This one had me: SwiftF0's +9.4 ms looked exactly like a corpus
  offset until an instrument sharing nothing with the first one said otherwise.)

What is actually happening is the **envelope**. Inside a symmetric window straddling a
decaying note, the energy is weighted toward the note's earlier, louder part, so an
energy-weighted estimator reports what the signal was doing slightly in the *past*. It is
late, and the wider its window, the later — with no group delay in it anywhere. The probe
demonstrates it on synthetic audio where the truth is analytic and there is nothing left to
be offset:

| envelope | lag | note |
|---|---|---|
| flat amplitude | **+0.01 ms** | identical pitch trajectory |
| note attack + decay | **+5.11 ms** | *only* the envelope differs |

Real music is made of decaying notes, so **every** estimator on this corpus pays this,
including the bank. A peak is therefore `group_delay + envelope_bias`, and only the first
term belongs to the detector.

What this does and does not change:

- **Does not change the numbers.** Both columns are still the numbers they were, and
  comparing two detectors *each at its own peak* is still fair — the envelope term is
  common to both. The RT-SWIPE gate (`R2`) is unaffected.
- **Does change what the peak means.** The bank's own group delay is `peak − its envelope
  bias`, and that bias is not yet measured — the +5.11 ms above is a *symmetric* 64 ms
  window's, while the bank's window is a causal IIR decay of a different shape and width. So
  the peak (16 ms on the corpus) is an upper bound on the bank's delay, not a measurement of
  it, and "it landed inside 8–29" was never evidence: a 21 ms-wide range accepts almost
  anything.
- **The way to measure the bank's delay** is `resonator::bank_latency_probe`, which drives
  a step and watches the output. That one is a real latency measurement. The corpus sweep
  is not, and should stop being quoted as a second opinion on it.

## What it reports, and why each line is there

- **RPA** — voiced frames scored within 50 cents. *The* number; the same metric
  (`mir_eval`'s) the literature reports, reproduced rather than tuned.
- **octave errors** — errors whose residue on the octave circle is within tolerance.
  Split out because an octave miss and a rubbish miss argue for **different fixes**: a
  Viterbi can outvote a lone octave excursion, but it cannot invent a candidate the
  salience never proposed. (On this corpus an octave *is* an error. On `testdata/` it is
  not necessarily — `g_open_real_octave.wav` is the player actually playing the octave.
  See the memory note on that trap.)
- **no pitch on voiced** — the scorer declined a frame that has a pitch.
- **other errors** — neither correct, octave, nor silent. Where the next bug hides.
- **mean |cents| when correct** — fine-pitch quality *given* the note was right.
- **pitch in silence** — pitch crowned where the annotation says unvoiced. Not part of
  RPA (voiced-only), reported because hearing notes in silence is a different defect from
  mis-hearing them.

## The external oracle — the check on the checker

Everything above is our code measuring our detector. Until 2026-07-15 no number this
harness printed had ever been compared to anything outside this repository, which is a
problem no amount of internal care fixes: a benchmark that is wrong *in its own favour*
is worse than no benchmark.

So we run a reference implementation, feed its trajectory through our scorer, and require
the two harnesses to agree.

```sh
uv run tools/swiftf0_oracle.py       # ~10 min, writes datasets/oracle_swiftf0/
cargo test --release --lib external_rpa_on_corpus -- --ignored --nocapture
```

`tools/swiftf0_oracle.py` drives **SwiftF0** (MIT, 398 KB ONNX, ranked #1 in
[`lars76/pitch-benchmark`](https://github.com/lars76/pitch-benchmark)) through *that
repository's own harness*, pinned at commit `87982db2` and cloned on demand into the
git-ignored `datasets/reference/`. Their dataset loader, their algorithm wrapper, their
metric — imported, never reimplemented, because the entire point is to be checked by code
we did not write.

**Result: our scorer reproduces theirs exactly — 91.98 % vs 91.98 %, a delta of +0.00
points over 1.26 M frames.** Our RPA arithmetic is not the weak link in any number here.

Getting to that comparison meant establishing four things about their numbers first, each
of which independently breaks the naive "does our RPA match their published RPA?" check:

1. **Their `rpa` is not RPA.** `run_single_evaluation` builds the denominator as
   `pred_voicing & true_voicing`: a voiced frame the detector answered with *silence* is
   dropped from the denominator rather than counted wrong. That is accuracy conditional on
   the detector having spoken. mir_eval — and `Tally::rpa` — divide by every voiced frame.
   Their composite is still honest; it re-applies the penalty through a separate
   voicing-recall term. But the component, read alone, flatters. `Tally::rpa_conditional`
   exists solely to speak their dialect, and the summary prints both side by side.
2. **Their published 92.0 % for MDB-stem-synth is not RPA either** — it is a harmonic mean
   of six metrics (RPA, cents accuracy, voicing precision/recall, octave accuracy, gross
   error accuracy). Per-dataset RPA is never published; the only RPA they report (0.905)
   is aggregated across all eight datasets. That 90.2 % ≈ 0.905 coincidence is a trap.
3. **Their published numbers are measured on noise-augmented audio.** `pitch_benchmark.py`
   unconditionally wraps the corpus in `CHiMeNoiseDataset` — CHiME-Home plus Gaussian
   noise, SNR 10–30 dB, random voice gain ±6 dB. There is no clean path through their
   runner. Our oracle script skips the wrapper (its single deliberate deviation), so our
   93.96 % harmonic mean on clean audio sits about two points above their noisy 92.0 %,
   in the direction and of the size one would expect.
4. **Their threshold is oracle-tuned per dataset** — swept 0…1 in steps of 0.1, best
   kept. A ceiling, not a production setting. (0.9 wins on clean MDB, consistent with the
   0.887 baked into their wrapper.)

### The reference's annotation drifts — and it costs them 4.27 points

The gate scores against **their** resampled annotation (`datasets/oracle_swiftf0/truth/`),
not the corpus's. That is not a detail; it is what makes the comparison decidable.

`PitchDataset.process_sample` resamples the 2.9 ms annotation onto the frame grid with
`F.interpolate(mode="linear", align_corners=True)`. `align_corners` maps annotation index
`i` onto frame `i·(L−1)/(N−1)`, so on a 171-second track frame `j` reads its truth from
`0.0160013·j` seconds while the harness labels that frame `0.016·j`. The error is tiny per
frame and accumulates linearly: **~14 ms by the end of a long track, most of a whole
frame.** On 5.79 % of voiced frames their truth ends up more than 50 cents from the
corpus's own (worst case 946 cents), and every one of the worst frames sits in the tail of
its track.

Scored against the corpus annotation, SwiftF0 gets **96.25 %** by their own metric; against
their drifted annotation, **91.98 %**. The 4.27-point gap is not a scorer disagreement —
it is the reference marking a correct answer wrong because it is asking about the wrong
moment. (The first cut of this gate scored against the corpus annotation and reported our
number 3.12 points "above" theirs, which is how the drift was found. The hypothesis that
linear interpolation was blending the 0 Hz unvoiced markers into real pitches was tested
and rejected: only 18 of 389 corrupted frames sit on a voiced/unvoiced boundary.)

So the harness holds the annotation fixed at theirs and lets the scorers be the only
variable. Our own numbers keep using the corpus annotation, which is the accurate one.

### What the oracle says about SwiftF0 itself

Incidentally — not the point of the exercise, but the map for [the RT-SWIPE
track](pitch_detection_survey.md):

| | RPA (mir_eval convention, corpus annotation) |
|---|---|
| SwiftF0, lag 0 | 87.87 % |
| SwiftF0, lag +8 ms | **89.54 %** |

The sweep peaking at +8 ms — half a hop — rather than 0 is worth a look before SwiftF0 is
used to arbitrate anything about timing.

The gap between that and its 96.25 % under their conditional metric is one number:
**SwiftF0 declines 8.70 % of voiced frames outright**. Under mir_eval those are misses;
under theirs they vanish from the denominator. Same trajectory, nine points of headline.

## H0: отгружаемая линия, 2026-10-20

`melody_rpa_on_corpus` прогоняет WAV через общий `audio::test_rig::Rig`:
обе плоскости движка, pYIN-якорь, общий уровень, треллис и ремонтный слой.
Скорится `MelodyFrame::pitch` по собственному `t` из `melody_since`, до сегментатора.
Это линия пианоролла; сегменты стаффа этим тестом не оцениваются.
История читается курсором каждые 0.5 с, поэтому её ограничение в 2 с не теряет кадры.
Rig перенесён из тестов `core`, а не скопирован. Его callback — 10 мс;
публикация при настройке 16 мс получается каждые 20 мс на аудиочасах.
У отдельных фронтендов шаг 16 мс (при 44.1 кГц — 705 сэмплов).

Голос — все 60 стемов, скрипка — все 14; шум выключен, настройки движка стандартные,
кроме явно выбранного фронтенда. В таблице RPA в процентах; «лучший» означает
максимум **проверенной сетки**, у каждой строки свой лаг. Две полосы скорятся
одновременно; скрипка целиком внутри 65–2093 Гц.

| набор | сигнал | RPA лаг 0, вся полоса / 65–2093 Гц | лучший RPA, вся / 65–2093 | лаг, мс | октавы, % на лучшем лаге, 65–2093 |
|---|---|---|---|---|---|
| голос | банк, отдельный фронтенд | 84.43 / 84.44 | 95.55 / 95.56 | 16 | 0.28 |
| голос | банк, salience в движке | 89.21 / 89.23 | 95.39 / 95.40 | 16 | 0.27 |
| голос | **линия банка** | **73.69 / 73.71** | **83.17 / 83.18** | **32** | **0.12** |
| голос | RT-SWIPE, отдельный фронтенд | 84.92 / 84.94 | 96.61 / 96.63 | 16 | 0.10 |
| голос | RT-SWIPE, salience в движке | 84.97 / 84.98 | 96.62 / 96.63 | 16 | 0.10 |
| голос | **линия RT-SWIPE** | **71.92 / 71.93** | **89.22 / 89.24** | **32** | **0.04** |
| скрипка | банк, отдельный фронтенд | 87.12 / 87.12 | 89.68 / 89.68 | 16 | 1.46 |
| скрипка | банк, salience в движке | 87.74 / 87.74 | 89.50 / 89.50 | 16 | 1.37 |
| скрипка | **линия банка** | **81.71 / 81.71** | **85.18 / 85.18** | **64¹** | **0.90** |
| скрипка | RT-SWIPE, отдельный фронтенд | 94.01 / 94.01 | 96.48 / 96.48 | 8 | 1.00 |
| скрипка | RT-SWIPE, salience в движке | 94.03 / 94.03 | 95.16 / 95.16 | 16 | 0.95 |
| скрипка | **линия RT-SWIPE** | **89.11 / 89.11** | **92.39 / 92.39** | **32** | **0.87** |

¹ 64 мс — правая граница развёртки, не установленное положение пика.
Полная развёртка линии (65–2093 Гц):

| линия | 0 мс | 16 мс | 32 мс | 48 мс | 64 мс |
|---|---|---|---|---|---|
| голос, банк | 73.71 | 80.83 | 83.18 | 81.30 | 77.17 |
| голос, RT-SWIPE | 71.93 | 81.53 | 89.24 | 87.17 | 81.36 |
| скрипка, банк | 81.71 | 83.40 | 84.43 | 85.10 | 85.18 |
| скрипка, RT-SWIPE | 89.11 | 92.08 | 92.39 | 91.25 | 89.95 |

**Цепочка убирает часть октав, но теряет точность.** Относительно отдельного
фронтенда лучший RPA линии ниже на 12.38 / 7.39 п.п. на голосе (банк / RT-SWIPE)
и на 4.50 / 4.09 п.п. на скрипке; на лаге 0 потери — 10.73 / 13.01 и 5.41 / 4.90 п.п.
На голосе линия против salience того же движка теряет 12.22 / 7.39 п.п.
даже при выборе собственного лучшего лага: одной общей задержкой разрыв не объясняется.
На скрипке аналогичные потери — 4.32 / 2.77 п.п.
Это совокупный вклад треллиса, ремонтного слоя и гейта; эти измерения не выделяют
виновника внутри цепочки и не разрешают удалять ремонтный слой (§7 `note_detection.md`).
При этом на голосе молчание линии невелико (0.70 / 0.69% звучащих кадров при лаге 0),
а основной остаток на лучшем лаге — прочие ошибки высоты (16.30 / 10.37%).

Salience здесь — argmax положительной кривой `MelodyFrame::salience`, преобразованной
для показа монотонной нормировкой и gamma. Берутся **центры бинов**, без `resolve`
и уточнения по парциалам. Поэтому salience против отдельного фронтенда смешивает
вклад каденции и способ чтения высоты; это не чистая абляция каденции, а её дрожание
нельзя сравнивать с уточнённой линией. В частности, +4.79 п.п. на лаге 0 у банка
на голосе нельзя приписывать одной каденции.

Логи сессии 9 сохранены в `datasets/bench_runs/*_2026-10-20.log`:
`h1_{bank,rtswipe}_voice_snrinf`, `h1_melody_{bank,rtswipe}_voice_snrinf`,
`h1_{bank,rtswipe}_violin`, `h0_melody_{bank,rtswipe}_violin`,
`h0_stage_{bank,rtswipe}_{voice,violin}`. Фильтр теста банка в старой очереди
также запускал RT-SWIPE: для таблицы взят блок банка после завершения RT-SWIPE.
Все 34 лога очередей H0–H2 завершены без отказов (38 benchmark-тестов с учётом
этого совпадения фильтров); проверены число треков, обе полосы и конфигурации шума.
Последний отделённый прогон salience RT-SWIPE на скрипке завершился в 17:40;
его дождались через `wait_for.py`, повторных прогонов не запускали.
Полный корпус через движок остаётся отдельным пунктом плана; эту таблицу на него
не экстраполируем.

```sh
PITCH_BENCH_INSTRUMENT="male singer,female singer" PITCH_BENCH_FRONTEND=bank \
  cargo test --release --lib audio::dsp::pitch_bench::melody_rpa_on_corpus -- --exact --ignored --nocapture
# Второй фронтенд: PITCH_BENCH_FRONTEND=rtswipe; скрипка: PITCH_BENCH_INSTRUMENT=violin.
# Вход треллиса вместо линии: PITCH_BENCH_MELODY_STAGE=salience.
```

## H1: метрики спокойствия, 2026-10-20

`Tally::calm` считает октавные события, дрожание и дробление в том же проходе,
что RPA; `report` печатает их отдельно для каждой полосы и каждого лага.
Событие — максимальная серия октавных ошибок, её длительность — сумма аудиоинтервалов
кадров. Частота событий делится на минуты звучащей правды в этой полосе.
Кадр представляет интервал от предыдущего времени до собственного; первый — от начала
аудио, если его время больше нуля. Между треками серии и пары не соединяются.

Для дрожания оба соседних кадра детектора должны быть верными. Между соответствующими
ближайшими отсчётами **исходной аннотации**, с тем же лагом, каждый шаг должен быть
строго меньше 5 ¢, все отсчёты — звучащими и в полосе. Так короткая пауза или скачок
с возвратом между кадрами не попадают в выборку. Проверка идёт на сетке MDB 2.9 мс,
а не на шагах детекторов 8/16/20/40 мс. Медиана и p90 берутся из общей гистограммы
`|Δ оценки|` с бином 0.1 ¢ (печатается верхняя граница бина), рядом — число пар.
Величина остаётся движением за **собственный шаг детектора**, без нормировки на время;
плавное вибрато и глиссандо с шагом <5 ¢ тоже проходят этот критерий. Поэтому это
не изолированная ошибка трекера относительно неподвижного тона, и разные каденции
надо учитывать при сравнении.

Дробление — отношение **независимых** чисел звучащих серий оценки и правды
на сетке детектора, каждая ограничивается полосой. Число серий правды слегка
меняется с каденцией и лагом: редкая сетка может пропустить короткую паузу.
Ложное озвучивание может склеить два отрезка правды
или добавить лишний. Значение ниже 1 возможно при склейках и пропусках;
его нельзя читать как безусловное улучшение. В отчёте есть оба числа серий;
если серий правды нет, отношение не определено и печатается «—».

Чистый голос: все 60 стемов (41 мужской, 19 женский), 65–2093 Гц;
peak comb с гейтом −55 dBFS. «Лучший» лаг выбирается по RPA из штатной сетки,
а не по минимуму октав или дрожания. Внешняя строка SwiftF0 читается из
сохранённых CSV `datasets/oracle_swiftf0/`, без нового запуска модели.

| выход | шаг, мс | лучший лаг, мс | RPA, % | октавных событий/мин | средняя серия, мс | дрожание, мед./p90 ¢ | пар | серии оценки/правды = дробление |
|---|---|---|---|---|---|---|---|---|
| банк SWIPE′ | 16 | 16 | 95.56 | 8.77 | 19 | 5.1 / 15.8 | 208890 | 6661/4114 = 1.62 |
| RT-SWIPE | 16 | 16 | 96.63 | 2.58 | 23 | 10.4 / 13.6 | 213795 | 4840/4114 = 1.18 |
| решето | 16 | 24 | 94.95 | 15.94 | 21 | 5.4 / 16.1 | 213439 | 11405/4114 = 2.77 |
| peak comb, сглаженный | 8 | 64 | 94.00 | 0.44 | 34 | 2.3 / 7.4 | 453253 | 5540/4122 = 1.34 |
| peak comb, сырой | 8 | 48 | 92.00 | 0.67 | 19 | 2.6 / 8.0 | 450070 | 10261/4122 = 2.49 |
| pYIN | 40 | 72 | 74.60 | 1.30 | 184 | 5.0 / 19.3 | 69058 | 5957/4097 = 1.45 |
| линия H0, банк | 20 | 32 | 83.18 | 2.27 | 31 | 5.8 / 19.0 | 154300 | 2113/4111 = 0.51 |
| линия H0, RT-SWIPE | 20 | 32 | 89.24 | 1.08 | 25 | 0.4 / 12.8 | 160406 | 2301/4111 = 0.56 |
| SwiftF0, внешняя траектория | 16 | 8 | 92.56 | 0.11 | 53 | 2.8 / 15.4 | 209349 | 5412/4110 = 1.32 |

На **лаге 0**, с задержкой в оценке:

| выход | RPA, % | октавных событий/мин | средняя серия, мс | дрожание, мед./p90 ¢ | пар | дробление |
|---|---|---|---|---|---|---|
| банк SWIPE′ | 84.44 | 9.79 | 19 | 5.5 / 19.2 | 205955 | 1.62 |
| RT-SWIPE | 84.94 | 4.77 | 20 | 11.4 / 24.4 | 209204 | 1.18 |
| решето | 79.96 | 6.05 | 20 | 5.8 / 20.7 | 206008 | 2.77 |
| peak comb, сглаженный | 61.34 | 3.56 | 13 | 2.0 / 6.9 | 333324 | 1.34 |
| peak comb, сырой | 65.76 | 1.50 | 11 | 2.3 / 7.8 | 363660 | 2.49 |
| pYIN | 60.90 | 1.43 | 135 | 4.6 / 16.7 | 56479 | 1.45 |
| линия H0, банк | 73.71 | 2.88 | 29 | 5.6 / 18.7 | 144648 | 0.51 |
| линия H0, RT-SWIPE | 71.93 | 1.86 | 24 | 0.3 / 12.7 | 141624 | 0.56 |
| SwiftF0, внешняя траектория | 90.24 | 0.11 | 53 | 2.9 / 15.8 | 208905 | 1.32 |

**Гипотеза H1 в сильной форме не подтвердилась.** Сглаженный peak comb
имеет лучший измеренный p90 (7.4 ¢ на лучшем лаге), но медиана у линии
RT-SWIPE ниже (0.4 против 2.3 ¢), октавные события у SwiftF0 реже
(0.11 против 0.44/мин), а дробление у покадрового RT-SWIPE ближе к единице
(1.18 против 1.34). p90 считается за разные шаги, 8/16/20/40 мс;
он не отделяет собственное дрожание от движения вслед за плавной правдой.
На одной и той же каденции 8 мс сглаживание comb меняет p90 на своих лучших
лагах лишь с 8.0 до 7.4 ¢, зато число серий — с 10261 до 5540.
Преимущество по этим полям не отменяет цену задержки 64 мс и RPA 61.34%
на лаге 0 против 71.93% у линии RT-SWIPE.

Дробление линий H0 **0.51 / 0.56** (банк / RT-SWIPE) не означает
победу над дроблением comb 1.34: они выдают лишь 2113 / 2301 звучащую серию
против 4111 в правде, а высота в тишине на лаге 0 — **8.51 / 8.94%**.
Этот показатель надо читать вместе с ложным озвучиванием и RPA.

Логи этой выборки — `datasets/bench_runs/h1_s11_*_voice_snrinf_2026-10-20.log`;
банк запускался с полным именем теста и `--exact`, без повторного RT-SWIPE.
Оба фронтенда и линии используют стандартные настройки; полный корпус через
движок остаётся отдельным пунктом. Численный оракул на всех сохранённых CSV
(`h1_s11_external_oracle_2026-10-20.log`) сохранил 91.98% против 91.98% эталона,
разница +0.00 п.п. Пять итоговых release-тестов метрик и генератора шума прошли,
как и все десять измерительных прогонов (девять выходов на голосе и полный
сохранённый оракул). Все восемь внутренних голосовых прогонов покрывают 60 треков,
две полосы и пять лагов на 12 потоках по умолчанию; их развёртки RPA, октавных
и прочих ошибок совпадают с логами сессии 9. Внешняя голосовая строка тоже
покрывает все 60 стемов, в одной эталонной полосе.

Старые `h1_*_voice_snr{inf,20,10}_2026-10-20.log` сохранены: RPA и октавные
кадры пригодны, но дрожание в них отбиралось по шагу детектора, а серии оценки
ограничивались звучащей правдой. Исправленные шумовые измерения — ниже, H2;
чистая точка ∞ взята из H1 сессии 11.

## H2: белый шум на голосе, 2026-10-20

Все 60 голосовых стемов MDB, полоса 65–2093 Гц; шесть покадровых выходов
и две отгружаемые линии H0. Настройки и каденции те же, что в H1, comb
с гейтом −55 dBFS. `PITCH_BENCH_SNR_DB=20` или `10` добавляет к каждому
сэмплу гауссов белый шум с RMS = RMS всего файла / 10^(SNR/20).
LCG + Box–Muller, зерно FNV-1a от имени WAV: каждый детектор слышит один
и тот же шум. RMS включает паузы, поэтому SNR звучащих частей выше номинального;
это условие файла, а не измерение SNR только на голосе. Для чистой точки
переменную надо убрать из окружения. Внешняя траектория SwiftF0 из H1
получена на чистом аудио и в шумовое сравнение не входит.

«Лучший» — максимум RPA по штатной сетке лагов каждого выхода, отдельно
для каждого SNR. Это компенсация времени при оценке, а не измерение задержки.
Октавы — на лучшем лаге; молчание на звучащих и высота в тишине — на лаге 0.

| выход | SNR, дБ | RPA 0 / лучший, % | лучший лаг, мс | октавы, % | молчит, % | высота в тишине, % |
|---|---|---|---|---|---|---|
| банк SWIPE′ | ∞ | 84.44 / 95.56 | 16 | 0.28 | 0.01 | 8.58 |
| банк SWIPE′ | 20 | 80.86 / 91.33 | 16 | 1.31 | 0.00 | 99.99 |
| банк SWIPE′ | 10 | 68.20 / 76.40 | 16 | 5.58 | 0.00 | 99.99 |
| RT-SWIPE | ∞ | 84.94 / 96.63 | 16 | 0.10 | 0.04 | 8.07 |
| RT-SWIPE | 20 | 84.51 / 96.06 | 16 | 0.23 | 0.24 | 76.01 |
| RT-SWIPE | 10 | 82.53 / 93.45 | 16 | 0.63 | 0.60 | 75.73 |
| решето | ∞ | 79.96 / 94.95 | 24 | 0.56 | 4.91 | 1.84 |
| решето | 20 | 70.81 / 81.86 | 24 | 1.79 | 16.72 | 1.34 |
| решето | 10 | 54.65 / 61.22 | 24 | 2.76 | 36.18 | 1.40 |
| comb, сглаженный | ∞ | 61.34 / 94.00 | 64 | 0.03 | 4.75 | 4.39 |
| comb, сглаженный | 20 | 61.02 / 93.10 | 64 | 0.02 | 5.65 | 4.04 |
| comb, сглаженный | 10 | 58.82 / 87.90 | 64 | 0.02 | 10.43 | 2.75 |
| comb, сырой | ∞ | 65.76 / 92.00 | 48 | 0.02 | 8.35 | 2.38 |
| comb, сырой | 20 | 65.22 / 90.88 | 48 | 0.02 | 9.50 | 2.10 |
| comb, сырой | 10 | 62.06 / 84.69 | 48 | 0.04 | 15.33 | 1.18 |
| pYIN | ∞ | 60.90 / 74.60 | 72 | 0.40 | 18.09 | 5.33 |
| pYIN | 20 | 58.96 / 72.44 | 72 | 0.39 | 21.00 | 3.87 |
| pYIN | 10 | 50.69 / 62.11 | 96¹ | 0.35 | 33.73 | 2.00 |
| линия H0, банк | ∞ | 73.71 / 83.18 | 32 | 0.12 | 0.70 | 8.51 |
| линия H0, банк | 20 | 72.56 / 81.59 | 32 | 0.25 | 0.78 | 50.82 |
| линия H0, банк | 10 | 68.07 / 76.27 | 32 | 1.43 | 1.54 | 92.80 |
| линия H0, RT-SWIPE | ∞ | 71.93 / 89.24 | 32 | 0.04 | 0.69 | 8.94 |
| линия H0, RT-SWIPE | 20 | 70.24 / 86.44 | 32 | 0.16 | 1.66 | 38.26 |
| линия H0, RT-SWIPE | 10 | 68.31 / 82.78 | 48 | 0.31 | 2.22 | 69.99 |

¹ 96 мс — край проверенной сетки pYIN; положение максимума не установлено.

Метрики H1 на тех же лучших RPA-лагах; для октавных событий показан
также лаг 0. Средняя длина серии, дрожание и дробление — на лучшем лаге.
Дрожание измеряется за собственный шаг выхода, как в H1, только на верных
парах со стоящей правдой; число пар важно при росте отказов и ошибок.

| выход | SNR, дБ | октавных событий/мин, 0 / лучший | серия, мс | дрожание, мед./p90 ¢ | пар | серии оценки/правды = дробление |
|---|---|---|---|---|---|---|
| банк SWIPE′ | ∞ | 9.79 / 8.77 | 19 | 5.1 / 15.8 | 208890 | 6661/4114 = 1.62 |
| банк SWIPE′ | 20 | 38.77 / 41.09 | 19 | 5.2 / 16.2 | 197083 | 95801/4114 = 23.29 |
| банк SWIPE′ | 10 | 151.69 / 165.98 | 20 | 5.4 / 16.8 | 156585 | 106093/4114 = 25.79 |
| RT-SWIPE | ∞ | 4.77 / 2.58 | 23 | 10.4 / 13.6 | 213795 | 4840/4114 = 1.18 |
| RT-SWIPE | 20 | 6.97 / 5.45 | 26 | 11.4 / 15.2 | 212598 | 17755/4114 = 4.32 |
| RT-SWIPE | 10 | 15.75 / 15.28 | 25 | 11.9 / 24.4 | 207043 | 19471/4114 = 4.73 |
| решето | ∞ | 6.05 / 15.94 | 21 | 5.4 / 16.1 | 213439 | 11405/4114 = 2.77 |
| решето | 20 | 38.68 / 54.98 | 19 | 5.4 / 16.1 | 187290 | 19386/4114 = 4.71 |
| решето | 10 | 70.48 / 86.32 | 19 | 5.5 / 16.0 | 141333 | 25436/4114 = 6.18 |
| comb, сглаженный | ∞ | 3.56 / 0.44 | 34 | 2.3 / 7.4 | 453253 | 5540/4122 = 1.34 |
| comb, сглаженный | 20 | 3.11 / 0.34 | 40 | 2.3 / 7.4 | 450773 | 5715/4122 = 1.39 |
| comb, сглаженный | 10 | 2.22 / 0.40 | 31 | 2.3 / 7.4 | 432767 | 6472/4122 = 1.57 |
| comb, сырой | ∞ | 1.50 / 0.67 | 19 | 2.6 / 8.0 | 450070 | 10261/4122 = 2.49 |
| comb, сырой | 20 | 1.36 / 0.75 | 19 | 2.6 / 8.2 | 446942 | 10462/4122 = 2.54 |
| comb, сырой | 10 | 1.27 / 1.10 | 19 | 2.7 / 8.7 | 424995 | 11038/4122 = 2.68 |
| pYIN | ∞ | 1.43 / 1.30 | 184 | 5.0 / 19.3 | 69058 | 5957/4097 = 1.45 |
| pYIN | 20 | 1.23 / 1.26 | 184 | 5.0 / 19.4 | 67715 | 6076/4097 = 1.48 |
| pYIN | 10 | 0.99 / 1.06 | 199 | 4.8 / 19.0 | 60407 | 6204/4103 = 1.51 |
| линия H0, банк | ∞ | 2.88 / 2.27 | 31 | 5.8 / 19.0 | 154300 | 2113/4111 = 0.51 |
| линия H0, банк | 20 | 4.42 / 3.60 | 41 | 5.8 / 19.1 | 151867 | 1450/4111 = 0.35 |
| линия H0, банк | 10 | 12.42 / 11.86 | 72 | 5.9 / 19.6 | 141528 | 3243/4111 = 0.79 |
| линия H0, RT-SWIPE | ∞ | 1.86 / 1.08 | 25 | 0.4 / 12.8 | 160406 | 2301/4111 = 0.56 |
| линия H0, RT-SWIPE | 20 | 4.88 / 3.54 | 26 | 0.4 / 12.8 | 157256 | 4520/4111 = 1.10 |
| линия H0, RT-SWIPE | 10 | 7.30 / 4.98 | 37 | 0.5 / 12.9 | 155021 | 5249/4112 = 1.28 |

**Шум меняет рейтинг, но не лидера RPA.** На лучших лагах RT-SWIPE остаётся
первым (96.63 → 96.06 → 93.45%), а сглаженный comb поднимается с четвёртого
на второе место среди покадровых выходов: при 10 дБ он теряет 6.10 п.п.,
банк — 19.16, решето — 33.73. У решета растут и отказы (4.91 → 36.18%),
и октавы (0.56 → 2.76%); одна смена лага этого не исправляет.
Сравнение детекторов целиком не выделяет вклад отбеливания, HNR-гейта,
набора кандидатов и длинного окна comb; их причины требуют отдельных вариантов.

Устойчивый RPA RT-SWIPE на звучащих не означает устойчивое распознавание
тишины: уже при 20 дБ высота появляется на 76.01% беззвучных кадров.
У банка — на 99.99%. Цепочка H0 снижает эти доли, но при 10 дБ оставляет
92.80 / 69.99% (банк / RT-SWIPE). Её лучший RPA остаётся ниже фронтенда:
76.27 против 76.40% у банка и 82.78 против 93.45% у RT-SWIPE.
Это основание отдельно разбирать озвученность под шумом; таблица не разделяет
вклад гейта, треллиса, якоря и ремонта. Гейт и константы здесь не менялись.

Почти неизменный p90 не отменяет роста ошибок: у решета число верных пар
213439 → 141333 при октавных событиях 15.94 → 86.32/мин и дроблении
2.77 → 6.18; у банка дробление достигает 25.79. У сглаженного comb при
10 дБ — 0.40 события/мин, p90 7.4 ¢, дробление 1.57. Дробление линий
0.79/1.28 тоже нельзя читать отдельно от 92.80/69.99% высоты в тишине.

Чистая точка не повторялась: взяты восемь логов H1 сессии 11. При 20/10 дБ
все 16 release-прогонов покрывают по 60 треков, две полосы и пять лагов,
12 потоков по умолчанию; их RPA-развёртки совпадают с сессией 9.
Логи — `datasets/bench_runs/h2_s12_*_voice_snr{20,10}_2026-10-20.log`.
Первый незавершённый запуск банка прерван earlyoom, лог сохранён как
`.interrupted_earlyoom.log`; его повтор успешен, остальные готовые прогоны
не повторялись. Пять release-тестов метрик и шума passed, 0 failed.

```sh
PITCH_BENCH_INSTRUMENT="male singer,female singer" PITCH_BENCH_SNR_DB=20 \
  cargo test --release --lib audio::dsp::pitch_bench::sieve_rpa_on_corpus -- --exact --ignored --nocapture
# 10 дБ: PITCH_BENCH_SNR_DB=10; чисто: убрать PITCH_BENCH_SNR_DB.
# Остальные тесты — как в H1; линии: melody_rpa_on_corpus и PITCH_BENCH_FRONTEND=bank/rtswipe.
```

Белый шум не моделирует комнату, дыхание и согласные. Сравнение с CHiME
или живым голосом остаётся отдельным условием; дублей H3 пока нет.

## R1: детекторы из других приложений, 2026-10-20

По новому запросу Стаса разобраны Tuner, Tartini и Singing Carrots;
расчётные пути и первичные ссылки —
[`pitch_detection_survey.md`](pitch_detection_survey.md#разбор-приложений-по-запросу-стаса-2026-10-20).
Tuner и опубликованный WASM-фронтенд Singing Carrots реально запущены на
всех 60 голосовых и 14 скрипичных стемах. Tartini здесь численно не сравнивался.
Настройки приложений не подбирались на корпусе.

Tuner `961de776`: его Kotlin DSP компилируется без изменений, Android заменён
файловым вводом и заглушками логирования/ресурсов. Окно 4096, Tophat,
перекрытие 75%, диапазон 16–16000 Гц, штатные гейты и среднее по 5 значениям.
Сохранены и сырой выход коллектора, и `FrequencyEvaluatorSimple`.
`tools/tuner_app_probe.py` закрепляет исходники и зависимости, Android SDK не нужен.

Singing Carrots: вызов из опубликованного `tuner.js?v=1016` —
`Pitch("default", 4096, 1, sampleRate)`, по одному полному аудиоблоку за вызов.
Запускается сам опубликованный WASM через `tools/singing_carrots_probe.cjs`.
Проверка фактического интерфейса показала чтение всех 4096 сэмплов; значение
`1` в конструкторе нельзя трактовать как один сэмпл на обработанный блок.
На восьми контрольных гармонических блоках `default` и `yinfft` дали
одинаковые оценки, отличные от `yin`. В таблице оставлено имя фронтенда
приложения, без предположения о полном совпадении с текущим upstream aubio.
Браузерная отрисовка и удержание подписи ноты не эмулируются.

CSV — на собственных кадрах, время по правому краю полного окна, без
дописанных начальных кадров и без ресэмплинга оценки. Частота WAV — исходная
44.1 кГц; шаг Tuner 23.22 мс, Singing Carrots 92.88 мс, решета 16 мс.
Все траектории оцениваются тем же `score_frame` и метриками H1 через новый
`trajectory_rpa_on_corpus`: полоса 65–2093 Гц, лаги 0…128 мс с шагом 16 мс.
Строки решета — прежние штатные развёртки H1/H2 с лучшим лагом 24 мс.
Октавы — на лучшем RPA-лаге; молчание и высота в тишине — на лаге 0.

| выход | выборка | SNR, дБ | RPA 0 / лучший, % | лаг, мс | октавы, % | молчит / высота в тишине, % |
|---|---|---|---|---|---|---|
| Tuner, сырой | голос | ∞ | 67.01 / 83.64 | 48 | 2.31 | 0.00 / 100.00 |
| Tuner, готовый | голос | ∞ | 54.93 / 75.71 | 96 | 0.42 | 17.71 / 3.65 |
| Tuner, сырой | скрипка | ∞ | 88.27 / 92.32 | 48 | 2.75 | 0.00 / 100.00 |
| Tuner, готовый | скрипка | ∞ | 79.68 / 86.80 | 80 | 1.46 | 11.44 / 2.10 |
| Singing Carrots, фронтенд | голос | ∞ | 67.11 / 92.99 | 48 | 0.33 | 1.48 / 3.52 |
| Singing Carrots, фронтенд | голос | 20 | 66.99 / 93.26 | 48 | 0.41 | 1.84 / 3.56 |
| Singing Carrots, фронтенд | голос | 10 | 65.45 / 90.76 | 48 | 1.05 | 3.37 / 3.10 |
| Singing Carrots, фронтенд | скрипка | ∞ | 81.84 / 87.86 | 48 | 3.21 | 5.88 / 0.96 |
| решето | голос | ∞ | 79.96 / 94.95 | 24 | 0.56 | 4.91 / 1.84 |
| решето | голос | 20 | 70.81 / 81.86 | 24 | 1.79 | 16.72 / 1.34 |
| решето | голос | 10 | 54.65 / 61.22 | 24 | 2.76 | 36.18 / 1.40 |
| решето | скрипка | ∞ | 92.63 / 96.95 | 24 | 0.17 | 3.44 / 0.98 |

H1 на лучших RPA-лагах; шаги различаются, поэтому дрожание и длину минимального
октавного события надо читать с каденцией. Редкий шаг также пропускает часть
коротких пауз правды: у Singing Carrots на голосе 3728 серий против 4114 у решета.

| выход и выборка | SNR | октавных событий/мин | серия, мс | дрожание, мед./p90 ¢ | пар | серии оценки/правды = дробление |
|---|---|---|---|---|---|---|
| Tuner сырой, голос | ∞ | 37.03 | 37 | 4.0 / 15.5 | 130740 | 3810/4113 = 0.93 |
| Tuner готовый, голос | ∞ | 5.24 | 48 | 3.6 / 14.2 | 123350 | 13943/4112 = 3.39 |
| Tuner сырой, скрипка | ∞ | 26.80 | 62 | 1.6 / 8.0 | 106468 | 5799/2282 = 2.54 |
| Tuner готовый, скрипка | ∞ | 11.58 | 76 | 1.2 / 4.5 | 101391 | 5410/2283 = 2.37 |
| Singing Carrots, голос | ∞ | 2.07 | 95 | 15.2 / 50.9 | 25142 | 3251/3728 = 0.87 |
| Singing Carrots, голос | 20 | 2.57 | 96 | 15.4 / 51.0 | 25187 | 3464/3728 = 0.93 |
| Singing Carrots, голос | 10 | 6.31 | 100 | 15.9 / 51.8 | 24656 | 3859/3728 = 1.04 |
| Singing Carrots, скрипка | ∞ | 15.89 | 121 | 6.7 / 25.2 | 21194 | 1858/2040 = 0.91 |

**У приложения нашёлся выигрыш над решетом в шуме.** Singing Carrots при
20/10 дБ даёт +11.40/+29.54 п.п. лучшего RPA; при 10 дБ выигрывает и на лаге 0
на 10.80 п.п. Его 3.10% высоты в тишине при 10 дБ намного ниже 75.73% у RT-SWIPE,
хотя RT-SWIPE сохраняет более высокий лучший RPA 93.45% и более быстрый шаг.
На чистом голосе и скрипке оба проверенных приложения решету уступают.
Это кандидат для изучения шумозащиты, а не основание менять фронтенд приложения:
окна и каденции различны, длинное окно тоже может давать устойчивость.

Проверка задержки: 44.1 кГц, непрерывная фаза и постоянная амплитуда,
ступенька в 0.5 с; первый из трёх последовательных кадров в пределах 50 ¢ новой
ноты. Это одна привязка ступеньки к сетке, не худшая задержка по всем фазам.

| выход | A4→B4, мс | A4→A5, мс |
|---|---|---|
| Tuner сырой | 57.3 | 80.5 |
| Tuner готовый | 126.9 | 150.2 |
| Singing Carrots, фронтенд | 57.3 | 150.2 |

Шум передан без повторного округления: `dump_audio_for_external_detector`
выгружает `corpus_audio` во float32 WAV, включая **тот же** шум H2 и зерно
по исходному имени. Проверка чистого файла — 7560512 сэмплов совпали точно
с PCM16 после нормировки; CSV до/после float WAV тоже совпали. Новый путь
CSV воспроизводит все пять RPA-строк голосового SwiftF0 прежнего оракула;
полный сохранённый оракул — 91.98/91.98%, Δ+0.00 п.п.
Восемь измерительных release-тестов прошли, каждый покрывает 60 или 14 треков,
две полосы и девять лагов. Четыре синтетических контроля Tuner прошли, включая
отсутствующую фундаментальную; воспроизводятся `--probe`.

Логи — `datasets/bench_runs/r1_s12_*_2026-10-20.log`, CSV и параметры —
`datasets/app_tuner_961de776_clean/` и `datasets/app_singing_carrots_2026-10-20/`.
Публичные WASM/JS сохранены в `datasets/reference/singing_carrots_2026-10-20/`.

```sh
python tools/tuner_app_probe.py --instrument 'male singer,female singer' --output datasets/app_tuner_voice
python tools/tuner_app_probe.py --probe --output datasets/app_tuner_controls
PITCH_BENCH_INSTRUMENT='male singer,female singer' \
  PITCH_BENCH_TRAJECTORY_DIR=datasets/app_tuner_voice/line \
  cargo test --release --lib audio::dsp::pitch_bench::trajectory_rpa_on_corpus -- --exact --ignored --nocapture
# Аналогично для /raw и violin. Для Singing Carrots — tools/singing_carrots_probe.cjs.
# Его список WAV, JS и WASM задаются аргументами; run.json пишется после успешного захвата.
# Шум: отдельный release dump_audio_for_external_detector с PITCH_BENCH_SNR_DB=20/10
# и PITCH_BENCH_AUDIO_DUMP_DIR. При скоринге CSV переменную SNR убрать.
```

## R2: YINFFT при окне и шаге решета, 2026-10-20

Проверяется одна заранее выбранная конфигурация: окно 2048 отсчётов,
шаг `floor(sample_rate × 0.016)` (705 отсчётов, 15.986 мс при 44.1 кГц),
явный метод `yinfft` того же опубликованного WASM, что в R1. Порог и
частотное взвешивание внутри бинарника не менялись. Это проверка ядра при
коротком окне, ещё не собственная Rust-реализация и не фронтенд движка.
Штатный режим R1 4096/4096 остаётся доступен без дополнительных аргументов.

Все 60 голосовых и 14 скрипичных стемов: ∞/20/10 дБ, тот же гауссов шум H2
через float32 WAV, без квантования и клиппинга. Скоринг — существующий
`trajectory_rpa_on_corpus`, две полосы, фиксированные лаги
0/8/16/24/32/48/64/80/96 мс. Для решета на скрипке повторены три обычных
release-прогона, в том числе H1; голос решета взят из H1/H2 без повторов.
Ни окно, ни пороги на голосе или скрипке не подбирались.

Время YINFFT — правый край полного окна, первый кадр после 2048 сэмплов,
без начального дополнения нулями. У решета шаг тот же, но фаза кадров и
начальное заполнение отличаются. YINFFT сохраняет штатный поиск, решето
ищет 65–2093 Гц; ниже приведена эта полоса правды. Сетка и первые кадры
могут немного менять знаменатели H1. Лучший RPA-лаг не является задержкой
распознавания перехода.

Октавы — на лучшем RPA-лаге (24 мс у всех строк), молчание и высота в тишине —
на лаге 0. Исходные 4096/4096 — таблица R1 выше.

| выход | выборка | SNR, дБ | RPA 0 / лучший, % | октавы, % | молчит / высота в тишине, % |
|---|---|---|---|---|---|
| YINFFT 2048 | голос | ∞ | 79.22 / 95.97 | 0.30 | 1.62 / 1.69 |
| YINFFT 2048 | голос | 20 | 79.37 / 96.29 | 0.37 | 1.41 / 5.05 |
| YINFFT 2048 | голос | 10 | 77.63 / 93.65 | 0.72 | 2.36 / 9.84 |
| решето | голос | ∞ | 79.96 / 94.95 | 0.56 | 4.91 / 1.84 |
| решето | голос | 20 | 70.81 / 81.86 | 1.79 | 16.72 / 1.34 |
| решето | голос | 10 | 54.65 / 61.22 | 2.76 | 36.18 / 1.40 |
| YINFFT 2048 | скрипка | ∞ | 85.78 / 90.07 | 1.89 | 6.25 / 0.35 |
| YINFFT 2048 | скрипка | 20 | 86.68 / 91.21 | 1.99 | 5.18 / 0.54 |
| YINFFT 2048 | скрипка | 10 | 85.08 / 89.72 | 4.32 | 3.61 / 3.50 |
| решето | скрипка | ∞ | 92.63 / 96.95 | 0.17 | 3.44 / 0.98 |
| решето | скрипка | 20 | 75.17 / 77.57 | 5.71 | 17.04 / 0.50 |
| решето | скрипка | 10 | 50.07 / 51.46 | 7.79 | 40.99 / 0.55 |

H1 на лучшем RPA-лаге 24 мс. Теперь шаг обоих выходов ~16 мс, но у YINFFT
фаза сетки другая. Голос решета — таблица H2 выше.

| выход | выборка | SNR, дБ | октавных событий/мин | серия, мс | дрожание, мед./p90 ¢ | пар | серии оценки/правды = дробление |
|---|---|---|---|---|---|---|---|
| YINFFT 2048 | голос | ∞ | 7.72 | 23 | 5.6 / 16.3 | 210561 | 5759/4110 = 1.40 |
| YINFFT 2048 | голос | 20 | 9.79 | 22 | 5.7 / 16.7 | 211122 | 16158/4110 = 3.93 |
| YINFFT 2048 | голос | 10 | 21.08 | 21 | 6.1 / 18.2 | 205588 | 33307/4110 = 8.10 |
| YINFFT 2048 | скрипка | ∞ | 30.52 | 37 | 2.4 / 9.7 | 147948 | 3665/2282 = 1.61 |
| YINFFT 2048 | скрипка | 20 | 33.17 | 36 | 2.6 / 10.0 | 149816 | 3863/2282 = 1.69 |
| YINFFT 2048 | скрипка | 10 | 98.66 | 26 | 3.7 / 12.9 | 144692 | 13126/2282 = 5.75 |
| решето | скрипка | ∞ | 4.55 | 23 | 2.4 / 9.6 | 162717 | 4490/2282 = 1.97 |
| решето | скрипка | 20 | 163.87 | 21 | 2.9 / 10.1 | 123134 | 9628/2282 = 4.22 |
| решето | скрипка | 10 | 224.44 | 21 | 3.2 / 10.7 | 74080 | 14698/2282 = 6.44 |

**Шумостойкость сохраняется при коротком окне и частом шаге.** При 10 дБ
лучший RPA YINFFT выше решета на 32.43 п.п. на голосе и 38.26 на скрипке;
без сдвига — на 22.98 и 35.01 п.п. Это преимущество не требует исходного
окна 4096 и шага 92.88 мс. Вариант 2048 лучше исходного R1 по RPA на всех
четырёх ранее измеренных сочетаниях выборки/шума, включая чистую скрипку,
но решето на чистой скрипке остаётся впереди на 6.88 п.п.

На чистом голосе октавных событий меньше (7.72 против 15.94/мин), дробление
ниже (1.40 против 2.77), дрожание почти прежнее. При 10 дБ число событий
тоже ниже (21.08 против 86.32), но дробление **хуже** (8.10 против 6.18):
возникает высота на 9.84% тишины против 1.40%. Переход к частому шагу не
сохраняет спокойствие исходной редкой линии по всем метрикам. На шумной
скрипке выигрыш RPA соседствует с 3.50% высоты в тишине против 0.55%.
Следующий перенос расчётного ядра требует численного оракула опубликованного
WASM и отдельной оценки озвученности; это ещё не решение о фронтенде по
умолчанию. Общий треллис, UI и константы решета здесь не менялись.

На синтетических контролях: прежние два CSV ступенек режима R1 после
расширения адаптера совпали побайтно. При окне 2048 на 64 гармонических
блоках `default == yinfft`, а переиспользованный объект дал ровно тот же
выход, что новый объект на каждом блоке: скрытая память предыдущей высоты
на этих контролях не обнаружена.

`tools/yinfft_step_probe.py` проверяет оба режима на чистом 440 Гц,
отсутствующей фундаментальной (880 + 1320 Гц) и двух переходах с постоянной
амплитудой и непрерывной фазой. Частота 44.1 кГц, переходы после 22050 +
256·k сэмплов, k = 0…15. Задержка — первый из трёх последовательных кадров
в пределах 50 ¢ новой ноты. Все последние десять кадров каждого двухсекундного
контроля должны быть в пределах 5 ¢ цели. Итого 34 сигнала × 2 режима;
диапазон ниже относится к 16 проверенным фазам, а не ко всем возможным.

| режим | A4→B4, мс | A4→A5, мс |
|---|---|---|
| исходный 4096/4096 | 57.3–144.4 | 80.5–167.6 |
| YINFFT 2048/~16 мс | 27.6–42.1 | 37.7–52.3 |

Шесть скорингов YINFFT и три прогона решета — 9 release-тестов passed,
0 failed, по две полосы и 9/5 лагов, 12 потоков по умолчанию; четыре
release-выгрузки шумного аудио тоже прошли. Логи —
`datasets/bench_runs/r2_*_2026-10-20.log`, CSV и параметры —
`datasets/app_yinfft_2048_2026-10-20/{voice,violin}_{clean,snr20,snr10}/`,
контроли — там же в `controls/` и `step_controls/`. Исходный режим R1
проверен без повторения корпусных прогонов. Временные шумные WAV удалены
после захвата; CSV и исходный публичный WASM сохранены.

```sh
node tools/singing_carrots_probe.cjs AUBIO_JS AUBIO_WASM OUTPUT TRACK_LIST 2048 0.016 yinfft
PITCH_BENCH_INSTRUMENT='male singer,female singer' \
  PITCH_BENCH_TRAJECTORY_DIR=OUTPUT PITCH_BENCH_LAGS_MS=0,8,16,24,32,48,64,80,96 \
  cargo test --release --lib audio::dsp::pitch_bench::trajectory_rpa_on_corpus -- --exact --ignored --nocapture
python tools/yinfft_step_probe.py --output datasets/yinfft_step_controls
# Для скрипки: PITCH_BENCH_INSTRUMENT=violin.
# Для 20/10 дБ сначала dump_audio_for_external_detector, как в R1.
# При скоринге CSV PITCH_BENCH_SNR_DB должен быть убран.
```

## H4: октавы решета и переходы, 2026-10-20

Сессия 13. Диагностика и a/b/c завершены; **порог приёмки не пройден**,
основной счёт решета сохранён. a/b/c выбираются явно, без изменения
других детекторов и линии движка. Проверены 60 голосовых и 14 скрипичных
стемов MDB, а также живая трель A; диапазон кандидатов и маска MDB 65–2093 Гц,
окно 2048 сэмплов при 44.1 кГц, шаг 705 сэмплов (~16 мс).

Определения переходов зафиксированы до прогона. У MDB есть f0, но нет
разметки нот; округление f0 до MIDI создало бы ложные смены на вибрато.
Поэтому считаются два приближения: **резкий переход** — соседние значения
аннотации отличаются на ≥50 ¢ или меняется озвученность; **движение** —
то же при ≥5 ¢, дополнение к критерию стоящей правды H1. Второе включает
быстрое вибрато, первое пропускает плавные смены. Частоты не округляются,
время границы — середина соседних меток (граница nearest-neighbour).
Берётся абсолютное расстояние от `frame_time − scoring_lag` до ближайшей
границы. При отсутствии границ в файле есть отдельная корзина `no change`.
Границы строятся по исходной f0 без маски; знаменатель каждой строки —
звучащая правда в маске. Вместе с ошибками считаются все звучащие кадры,
чтобы отделить частоту ошибок от частоты самого соседства с переходами.

Голос baseline, **резкие переходы**; лаг 0 — 320888 звучащих кадров,
лаг 24 — 320865. «Вниз/вверх» включают все целые кратные октавы,
не только ±12 полутонов.

| расстояние, мс | октавных кадров, лаг 0 | звучащих, лаг 24 | вниз, лаг 24 | вверх, лаг 24 | октав / звучащих, %, лаг 24 |
|---|---|---|---|---|---|
| ≤8 | 28 | 8510 | 176 | 43 | 2.5734 |
| >8..24 | 66 | 10244 | 239 | 38 | 2.7040 |
| >24..48 | 93 | 13851 | 117 | 31 | 1.0685 |
| >48..96 | 67 | 24290 | 121 | 34 | 0.6381 |
| >96 | 403 | 263970 | 811 | 196 | 0.3815 |
| всего | 657 | 320865 | 1464 | 342 | 0.5629 |

Те же кадры относительно **движения**:

| расстояние, мс | октавных кадров, лаг 0 | звучащих, лаг 24 | вниз, лаг 24 | вверх, лаг 24 | октав / звучащих, %, лаг 24 |
|---|---|---|---|---|---|
| ≤8 | 308 | 106562 | 1344 | 158 | 1.4095 |
| >8..24 | 179 | 35667 | 62 | 81 | 0.4009 |
| >24..48 | 83 | 32326 | 20 | 44 | 0.1980 |
| >48..96 | 38 | 40582 | 14 | 32 | 0.1134 |
| >96 | 49 | 105728 | 24 | 27 | 0.0482 |

В корзине ≤8 мс от движения — **83.17% октав** при **33.21% звучащих
кадров**; при лаге 0 — 46.88% при 33.22%. У резкого перехода ≤24 мс
при лаге 24 находятся 27.46% октав при 5.84% кадров, но 55.76% октав
дальше 96 мс от резкого перехода. Рост октав при компенсации лага
связан с движением правды, однако это не доказательство единственной
причины «хвост окна»: плавные смены и ошибки самого счёта здесь не
разделены. 81.06% ошибок на лаге 24 направлены вниз.

Варианты из плана: a — лучший положительный вклад пика в пяти
межгармонических ячейках 0.5/1.5/2.5/⅓/⅔·f0, веса 1.8/1/1/1/1,
вычитается из `predicted`; ширина и положительная часть ядра — от
существующего `mesh` на частоте ячейки. b — только недобор k=1 стоит ×2;
c — оба изменения. Веса, нормировка, озвученность и временная сетка
не подбирались по результатам.

Чистый голос, все лучшие RPA на лаге 24 мс; H1 на этом же лаге,
молчание и высота в тишине — на лаге 0:

| режим | RPA 0 / лучший, % | октавы, % | событий/мин; серия, мс | дрожание мед./p90 ¢; пар | дробление, серии оценки/правды | молчит / высота в тишине, % |
|---|---|---|---|---|---|---|
| baseline | 79.96 / 94.95 | 0.56 | 15.94; 21 | 5.4/16.1; 213439 | 2.77, 11405/4114 | 4.91 / 1.84 |
| a | 79.96 / 94.92 | 0.52 | 15.18; 21 | 5.4/16.1; 213458 | 2.79, 11490/4114 | 4.99 / 1.82 |
| b | 79.98 / 95.01 | 0.26 | 8.26; 19 | 5.4/16.1; 213483 | 2.80, 11517/4114 | 5.14 / 1.80 |
| c | 79.98 / 94.99 | 0.21 | 6.85; 19 | 5.4/16.1; 213512 | 2.82, 11586/4114 | 5.22 / 1.79 |

Чистая скрипка, также лучший лаг 24 мс:

| режим | RPA 0 / лучший, % | октавы, % | событий/мин; серия, мс | дрожание мед./p90 ¢; пар | дробление, серии оценки/правды | молчит / высота в тишине, % |
|---|---|---|---|---|---|---|
| baseline | 92.63 / 96.95 | 0.17 | 4.55; 23 | 2.4/9.6; 162717 | 1.97, 4490/2282 | 3.44 / 0.98 |
| a | 92.69 / 96.98 | 0.09 | 3.10; 17 | 2.4/9.6; 162845 | 1.98, 4529/2282 | 3.50 / 0.98 |
| b | 92.63 / 96.94 | 0.15 | 3.86; 24 | 2.4/9.6; 162734 | 1.97, 4488/2282 | 3.48 / 0.98 |
| c | 92.70 / 96.98 | 0.07 | 2.23; 18 | 2.4/9.6; 162866 | 1.98, 4524/2282 | 3.54 / 0.98 |

Шум H2 — тот же детерминированный гауссов шум по RMS всего файла;
baseline 20/10 дБ взят из H2/R2 сессии 12, готовые прогоны не повторялись.
Во всех шумовых строках лучший RPA также на лаге 24 мс; H1 и октавы —
там же, молчание и высота в тишине — на лаге 0. Число пар и абсолютные
счётчики серий сохранены в логах.

Голос:

| режим, SNR дБ | RPA 0 / лучший, % | октавы, % | событий/мин; серия, мс | дрожание мед./p90 ¢ | дробление | молчит / высота в тишине, % |
|---|---|---|---|---|---|---|
| baseline, 20 | 70.81 / 81.86 | 1.79 | 54.98; 19 | 5.4/16.1 | 4.71 | 16.72 / 1.34 |
| baseline, 10 | 54.65 / 61.22 | 2.76 | 86.32; 19 | 5.5/16.0 | 6.18 | 36.18 / 1.40 |
| a, 20 | 70.69 / 81.69 | 1.75 | 54.31; 19 | 5.4/16.1 | 4.70 | 16.91 / 1.30 |
| a, 10 | 54.46 / 60.98 | 2.71 | 84.65; 19 | 5.5/16.0 | 6.13 | 36.48 / 1.36 |
| b, 20 | 71.12 / 82.25 | 0.85 | 28.24; 18 | 5.4/16.1 | 4.40 | 17.27 / 1.01 |
| b, 10 | 55.21 / 61.84 | 1.25 | 42.13; 18 | 5.5/16.0 | 5.44 | 37.09 / 0.78 |
| c, 20 | 70.99 / 82.08 | 0.82 | 27.42; 18 | 5.4/16.1 | 4.39 | 17.48 / 0.98 |
| c, 10 | 55.01 / 61.60 | 1.21 | 40.85; 18 | 5.5/16.1 | 5.42 | 37.38 / 0.76 |

Скрипка:

| режим, SNR дБ | RPA 0 / лучший, % | октавы, % | событий/мин; серия, мс | дрожание мед./p90 ¢ | дробление | молчит / высота в тишине, % |
|---|---|---|---|---|---|---|
| baseline, 20 | 75.17 / 77.57 | 5.71 | 163.87; 21 | 2.9/10.1 | 4.22 | 17.04 / 0.50 |
| baseline, 10 | 50.07 / 51.46 | 7.79 | 224.44; 21 | 3.2/10.7 | 6.44 | 40.99 / 0.55 |
| a, 20 | 74.97 / 77.37 | 5.61 | 161.74; 21 | 2.9/10.1 | 4.22 | 17.35 / 0.49 |
| a, 10 | 49.82 / 51.19 | 7.59 | 219.42; 21 | 3.2/10.6 | 6.39 | 41.46 / 0.53 |
| b, 20 | 77.11 / 79.57 | 3.11 | 98.45; 19 | 2.9/10.0 | 4.15 | 17.68 / 0.45 |
| b, 10 | 52.39 / 53.79 | 4.19 | 132.49; 19 | 3.3/10.7 | 6.00 | 42.29 / 0.36 |
| c, 20 | 76.87 / 79.31 | 3.06 | 96.82; 19 | 2.9/10.0 | 4.15 | 18.01 / 0.44 |
| c, 10 | 52.06 / 53.45 | 4.06 | 128.83; 19 | 3.3/10.6 | 5.98 | 42.77 / 0.35 |

b переносит выигрыш в октавах на оба инструмента и оба уровня шума;
a немного снижает RPA, c на голосе теряет часть выигрыша b по RPA.
Даже b оставляет голос при 10 дБ на 61.84% против 93.45% RT-SWIPE:
этот штраф не решает общую шумовую уязвимость решета.

Приёмка требует на голосе ≤0.1% октав и снижения событий, RPA ≥94.65%,
сохранения слабой/отсутствующей фундаментальной, трели A и задержки.
Ни один a/b/c не достигает первого порога. Все 10 тестов решета прошли
в каждом режиме, включая оба теста фундаментальной. На живой трели A
октав вниз baseline/a/b/c — 46/5287 (0.8701%), 52/5239 (0.9926%),
41/5278 (0.7768%), 48/5233 (0.9173%); ни один режим не возвращает
прежние 10%, но a/c слегка ухудшают абсолютный счётчик. Диапазон этого
отдельного прогона 174.6–2794 Гц, окно 1024, шаг 256; правда — первая
позиция 69–76 ±½ тона, знаменатель — звучащие оценки, не истинное время
голоса. Подробные счётчики — [`harmonic_sieve.md`, H4](harmonic_sieve.md#h4-октавы-у-переходов-и-три-штрафа-2026-10-20).

Синтетическая задержка: 12 гармоник 1/k, 44.1 кГц, окно 2048,
шаг 705, 16 равномерных фаз начала в одном шаге после 10 шагов прогрева;
после перехода ещё 10 шагов. Первый верный (±50 ¢) кадр должен оставаться верным до
конца отрезка. A4→B4: 22.99–37.98 мс во всех режимах, все фазы совпали;
A4→A5: baseline/b 28.98–50.98 мс, a/c 32.99–50.98 мс. В двух из
16 фаз a/c задерживают правильную октаву на один шаг, 15.99 мс;
b совпадает с baseline по всем 32 фазам. Это проверка реакции на две
ступеньки, не общая групповая задержка и не вывод только из окна.

Воспроизведение (полные развёртки 0/8/16/24/32 мс — в логах):

Логи — `datasets/bench_runs/h4_s13_*_2026-10-20.log`: 20 новых
release-прогонов (8 чистых + 12 шумовых) прошли, по 10 тестов решета
в четырёх режимах, 6 тестов стенда (включая диагностику) и 8 проб
трели/ступенек — passed. Численный оракул на всех 230 сохранённых CSV
дал 91.98%, Δ+0.00 п.п.; это скоринг готовых траекторий, не прогон
корпуса через движок. Все 20 строк развёрток RPA/H1 baseline на голосе
совпали с H1 сессии 11, на скрипке — с R2 сессии 12. Временные файлы
прогонов удалены; логи сохранены вне git.

```sh
PITCH_BENCH_INSTRUMENT='male singer,female singer' \
  PITCH_BENCH_SIEVE_VARIANT=b PITCH_BENCH_SIEVE_DIAGNOSTICS=1 \
  cargo test --release --lib audio::dsp::pitch_bench::sieve_rpa_on_corpus -- --exact --ignored --nocapture
# baseline/a/b/c; скрипка: PITCH_BENCH_INSTRUMENT=violin; шум: PITCH_BENCH_SNR_DB=20/10.
PITCH_BENCH_SIEVE_VARIANT=b cargo test --release --lib audio::dsp::sieve::tests -- --nocapture
PITCH_BENCH_SIEVE_VARIANT=b cargo test --release --lib h4_live_a_trill -- --ignored --nocapture
PITCH_BENCH_SIEVE_VARIANT=b cargo test --release --lib h4_step_latency -- --ignored --nocapture
```

## What it deliberately does not do

- **No annotation interpolation.** Nearest neighbour only. The corpus grid (2.9 ms) is
  ~5× finer than the bank's hop (16 ms), so the nearest sample is within 1.5 ms; inventing
  a trajectory between annotations would put made-up numbers in a benchmark's denominator.
- **No re-implementation.** It drives the production path — real bank, real snapshot,
  `ResonatorSnapshot::fundamental` for the frontend baseline, and the shared engine
  `Rig` → `melody_since` for the shipped line — the same discipline as `real_violin_g_probe`.
- **No silent lag correction.** The sweep reports lag 0 first and leads the summary with
  it. A single delay-compensated figure would be a better-looking number for a detector
  nobody runs.

## For scale (Marttila & Reiss 2025, Tables 1/3/5, MDB-stem-synth unless noted)

| Method | RPA | at 0 dB SNR (MIR-1K) | at −10 dB (MIR-1K) |
|---|---|---|---|
| SWIPE (their impl) | 96.1 % | 91.2 % | 75.2 % |
| SWIPE-tiny (647 params) | 96.5 % | 95.3 % | 88.5 % |
| PESTO (28.9k) | 94.6 % | 92.9 % | 81.7 % |
| pYIN | 91.6 % | 95.1 % | 85.8 % |

Read the noise columns before reaching for the network: on clean audio the 647-parameter
Toeplitz layer is worth +0.4 points over plain SWIPE. Its real purchase is robustness.
