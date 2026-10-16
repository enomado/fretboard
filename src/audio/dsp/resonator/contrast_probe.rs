//! Probe: does any fixed `alpha`/`beta` scale beat 1.0 on real music, and would an
//! *adaptive* one?
//!
//! The question behind it (user, 2026-10-16): on real music every move of the bank's
//! `alpha`/`beta` sliders away from 1 makes the display less contrasty — can the bank be
//! steered adaptively instead, by how much clean signal there is against junk?
//!
//! "Contrast" is measured as **sparsity** — the Rényi entropy of order 3 of the energy
//! distribution, reported as the *effective number of cells* `2^H₃` it occupies (lower is
//! sparser = sharper). This is the standard objective of adaptive time–frequency
//! representations (Baraniuk, Flandrin & Michel 2001; Jones & Baraniuk's adaptive kernel):
//! it rewards energy concentrated in few cells and is blind to the overall level.
//!
//! Two scopes, because `alpha` trades one axis for the other:
//! - **frame** — one column, frequency concentration only. What one spiral frame shows.
//!   A longer ring (small `alpha`) always wins here on stationary tones, so on its own
//!   this metric is biased toward small `alpha`.
//! - **block** — `BLOCK_FRAMES` columns normalised *together*, so time smear is paid for
//!   too. What the waterfall shows, and the fair judge of the trade.
//!
//! And the adaptivity oracle: per block, pick whichever `alpha` (beta = 1) is sparsest and
//! sum. If that oracle gains little over a fixed 1.0, steering `alpha` adaptively cannot
//! buy contrast either — the gain has to come from the display mapping.
//!
//! Run (release — the bank is per-sample IIR, debug is ~20× slower):
//!
//! ```text
//! CONTRAST_CORPUS=~/t/scratch/fretboard_contrast/corpus \
//!   cargo test --release --lib contrast_probe -- --ignored --nocapture
//! ```
//!
//! The corpus is any directory of mono 16-bit WAVs. Nothing here asserts: it is a
//! measurement, and its verdict lives in `docs/resonator_contrast.md`.

use std::path::{
    Path,
    PathBuf,
};

use super::{
    ResonatorAnalyzer,
    ResonatorViewSettings,
    raw_column,
};
use crate::audio::sample_rate::SampleRate;

/// Publish cadence of the probe — the shipped default `update_ms`.
const HOP_SECONDS: f32 = 0.016;
/// Frames dropped at the start: the bank's low bins ring up for hundreds of ms.
const WARMUP_SECONDS: f32 = 0.5;
/// ~0.5 s at the 16 ms hop — the span a waterfall glance takes in.
const BLOCK_FRAMES: usize = 32;
/// The scales swept. Geometric around 1, the slider's own (log) geometry.
const SCALES: [f32; 7] = [0.25, 0.5, 0.71, 1.0, 1.41, 2.0, 4.0];

/// Rényi entropy of order 3 of `energy` (any non-negative weights), as the effective
/// number of cells `2^H₃ = (Σq³)^(−1/2)`, `q = e/Σe`. `None` for an all-zero input — a
/// silent frame has no distribution to be sparse about.
fn effective_cells(energy: impl Iterator<Item = f32> + Clone) -> Option<f32> {
    let total: f64 = energy.clone().map(|e| e as f64).sum();
    if total <= 0.0 {
        return None;
    }
    let cube: f64 = energy.map(|e| (e as f64 / total).powi(3)).sum();
    Some(cube.powf(-0.5) as f32)
}

fn read_wav(path: &Path) -> (SampleRate, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().channels, 1, "{path:?}: corpus must be mono");
    let sample_rate = SampleRate(reader.spec().sample_rate);
    let samples = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
    (sample_rate, samples)
}

/// Raw (pre-display) reassigned columns of `samples` through a bank at the given scales,
/// one per `HOP_SECONDS`, warm-up dropped. Energy = magnitude² regardless of
/// `settings.power` (the probe runs magnitude mode, the shipped default).
fn columns(sample_rate: SampleRate, samples: &[f32], alpha: f32, beta: f32) -> Vec<Vec<f32>> {
    let mut analyzer = ResonatorAnalyzer::new(sample_rate);
    let settings = ResonatorViewSettings {
        alpha_scale: alpha,
        beta_scale: beta,
        ..ResonatorViewSettings::default()
    };
    analyzer.sync_settings(settings);
    let hop = (HOP_SECONDS * sample_rate.hz()) as usize;
    let warmup = (WARMUP_SECONDS / HOP_SECONDS) as usize;
    samples
        .chunks_exact(hop)
        .enumerate()
        .filter_map(|(index, chunk)| {
            analyzer.process_samples(chunk, true);
            (index >= warmup)
                .then(|| raw_column(&analyzer.bank, &analyzer.settings, &analyzer.detuning_hz, true))
        })
        .collect()
}

/// Per-track sparsity at one `(alpha, beta)`.
struct Sparsity {
    /// Mean effective cells of one column (frequency only).
    frame:  f32,
    /// Effective cells of each `BLOCK_FRAMES` block (time × frequency), in block order;
    /// `None` for a silent block, which has no distribution to judge.
    blocks: Vec<Option<f32>>,
}

fn sparsity(columns: &[Vec<f32>]) -> Sparsity {
    let frames: Vec<f32> = columns
        .iter()
        .filter_map(|column| effective_cells(column.iter().map(|m| m * m)))
        .collect();
    let blocks = columns
        .as_chunks::<BLOCK_FRAMES>()
        .0
        .iter()
        .map(|block| effective_cells(block.iter().flatten().map(|m| m * m)))
        .collect();
    Sparsity {
        frame: frames.iter().sum::<f32>() / frames.len() as f32,
        blocks,
    }
}

fn geometric_mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0f64, 0usize), |(s, n), v| (s + (v as f64).ln(), n + 1));
    (sum / count as f64).exp() as f32
}

#[test]
#[ignore = "a probe, not a check: needs CONTRAST_CORPUS — see the module doc"]
fn contrast_probe() {
    let corpus = PathBuf::from(std::env::var("CONTRAST_CORPUS").expect("set CONTRAST_CORPUS"));
    let mut tracks: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    tracks.sort();
    assert!(!tracks.is_empty(), "no .wav in {corpus:?}");

    // (alpha, beta) configurations: each axis alone with the other at 1, and the diagonal
    // (both sliders moved together — the slider pair's "resolution" gesture).
    let mut configs: Vec<(f32, f32)> = SCALES.iter().map(|&a| (a, 1.0)).collect();
    configs.extend(SCALES.iter().filter(|&&b| b != 1.0).map(|&b| (1.0, b)));
    configs.extend(SCALES.iter().filter(|&&s| s != 1.0).map(|&s| (s, s)));

    let results: Vec<(String, Vec<Sparsity>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = tracks
            .iter()
            .map(|path| {
                let configs = &configs;
                scope.spawn(move || {
                    let (sample_rate, samples) = read_wav(path);
                    let per_config = configs
                        .iter()
                        .map(|&(alpha, beta)| sparsity(&columns(sample_rate, &samples, alpha, beta)))
                        .collect();
                    (
                        path.file_stem().unwrap().to_string_lossy().into_owned(),
                        per_config,
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let baseline = configs.iter().position(|&c| c == (1.0, 1.0)).unwrap();
    println!("\n=== effective cells relative to (alpha, beta) = (1, 1); < 1.00 = sparser ===");
    println!("scope: F = frame (frequency only), B = 0.5 s block (time × frequency)");
    print!("{:<10}", "track");
    for (alpha, beta) in &configs {
        print!(" {:>9}", format!("{alpha}/{beta}"));
    }
    println!();
    for (name, per_config) in &results {
        let base = &per_config[baseline];
        let base_block = geometric_mean(base.blocks.iter().flatten().copied());
        print!("{:<8} F", name);
        for s in per_config {
            print!(" {:>9.2}", s.frame / base.frame);
        }
        println!();
        print!("{:<8} B", "");
        for s in per_config {
            print!(
                " {:>9.2}",
                geometric_mean(s.blocks.iter().flatten().copied()) / base_block
            );
        }
        println!();
    }

    // The adaptivity oracle over the alpha axis (beta = 1): per block, the sparsest alpha.
    println!("\n=== oracle: best alpha per 0.5 s block (beta = 1) ===");
    let alpha_axis: Vec<usize> = (0..SCALES.len()).collect(); // configs[0..7] are (a, 1)
    for (name, per_config) in &results {
        let blocks = per_config[baseline].blocks.len();
        let mut wins = vec![0usize; SCALES.len()];
        let mut oracle = Vec::with_capacity(blocks);
        let mut fixed_blocks = Vec::with_capacity(blocks);
        for b in 0..blocks {
            // Silent at 1.0 ⇒ silent at every scale (same input): skip the block entirely,
            // so the oracle and the fixed baseline average over the same set.
            let Some(fixed_value) = per_config[baseline].blocks[b] else {
                continue;
            };
            let (best, value) = alpha_axis
                .iter()
                .map(|&c| (c, per_config[c].blocks[b].unwrap()))
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .unwrap();
            wins[best] += 1;
            oracle.push(value);
            fixed_blocks.push(fixed_value);
        }
        let fixed = geometric_mean(fixed_blocks.into_iter());
        let gain = geometric_mean(oracle.iter().copied()) / fixed;
        let histogram: Vec<String> = SCALES
            .iter()
            .zip(&wins)
            .map(|(scale, n)| format!("{scale}:{n}"))
            .collect();
        println!(
            "{name:<10} oracle/fixed = {gain:.3}   wins {}",
            histogram.join(" ")
        );
    }
}

// ---------------------------------------------------------------------------------------
// Probe 2: against TRUTH. Sparsity is a proxy — it cannot tell a sharp partial from a sharp
// artefact. MDB-stem-synth can: every stem is a resynthesis with a perfect f0, so summing
// the stems of one song gives real polyphonic music whose every partial sits at a known
// `k·f0`. "Junk" is then everything the display lights *off* those partials — the bank's
// skirts, reverb tails, leakage — and that is exactly what the user calls lost contrast.
// ---------------------------------------------------------------------------------------

/// The bank's delay against the annotation — the lag-sweep peak (`pitch_bench`, 16 ms).
const TRUTH_LAG_SECONDS: f32 = 0.016;
/// A cell is "on a partial" within this distance of `k·f0`. The reassigned grid is 1/8
/// semitone, so this is ±2 cells — tight, because reassignment puts energy at the true
/// frequency rather than at a bin centre.
const TRUTH_TOLERANCE_SEMITONES: f32 = 0.25;
/// Partials counted per stem. Resynthesized stems carry a harmonic series; past ~16 the
/// partials of most instruments are below anything a display would show anyway.
const MAX_HARMONIC: usize = 16;
/// The excerpt of each song: past intros, long enough for every stem to have played.
const SEGMENT_SECONDS: (f32, f32) = (20.0, 50.0);

struct Song {
    name:        String,
    sample_rate: SampleRate,
    /// The stems summed over `SEGMENT_SECONDS`, peak-normalised.
    mix:         Vec<f32>,
    truths:      Vec<crate::audio::dsp::pitch_bench::Annotation>,
}

/// Every song with at least 3 stems, `CONTRAST_SONGS` of them (default 6), sorted.
fn polyphonic_songs() -> Vec<Song> {
    use crate::audio::dsp::pitch_bench::{
        load_annotation,
        read_wav,
        tracks,
    };
    let limit: usize = std::env::var("CONTRAST_SONGS").map_or(6, |v| v.parse().unwrap());
    let mut by_song: std::collections::BTreeMap<String, Vec<(PathBuf, PathBuf)>> = Default::default();
    for (id, wav, csv) in tracks() {
        let song = id.split("_STEM_").next().unwrap().to_string();
        by_song.entry(song).or_default().push((wav, csv));
    }
    by_song
        .into_iter()
        .filter(|(_, stems)| stems.len() >= 3)
        .take(limit)
        .map(|(name, stems)| {
            let mut sample_rate = None;
            let mut mix: Vec<f32> = Vec::new();
            let mut truths = Vec::new();
            for (wav, csv) in &stems {
                let (sr, samples) = read_wav(wav);
                assert_eq!(
                    *sample_rate.get_or_insert(sr),
                    sr,
                    "{name}: stems disagree on rate"
                );
                let from = (SEGMENT_SECONDS.0 * sr.hz()) as usize;
                let to = (SEGMENT_SECONDS.1 * sr.hz()) as usize;
                mix.resize(to - from, 0.0);
                // A stem shorter than the excerpt simply stops contributing.
                for (out, &s) in mix.iter_mut().zip(samples.iter().skip(from)) {
                    *out += s;
                }
                truths.push(load_annotation(csv));
            }
            let peak = mix.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.0, "{name}: silent excerpt");
            for s in &mut mix {
                *s *= 0.5 / peak;
            }
            Song {
                name,
                sample_rate: sample_rate.unwrap(),
                mix,
                truths,
            }
        })
        .collect()
}

/// Truth mask for column `index` (published after `(index + 1)` hops past warm-up):
/// `true` on cells within `TRUTH_TOLERANCE_SEMITONES` of a partial of a voiced stem.
/// `None` when no stem is voiced — a frame with nothing to be right about.
fn truth_mask(song: &Song, index: usize, len: usize, lo_midi: f32) -> Option<Vec<bool>> {
    let warmup = (WARMUP_SECONDS / HOP_SECONDS) as usize;
    let seconds = SEGMENT_SECONDS.0 + (index + warmup + 1) as f32 * HOP_SECONDS - TRUTH_LAG_SECONDS;
    let cells_per_semitone = super::OUTPUT_BINS_PER_SEMITONE as f32;
    let reach = (TRUTH_TOLERANCE_SEMITONES * cells_per_semitone).round() as isize;
    let mut mask = vec![false; len];
    let mut voiced = false;
    for truth in &song.truths {
        let Some(f0_midi) = truth.at(seconds) else {
            continue;
        };
        voiced = true;
        for k in 1..=MAX_HARMONIC {
            let midi = f0_midi + 12.0 * (k as f32).log2();
            let centre = ((midi - lo_midi) * cells_per_semitone).round() as isize;
            for cell in centre - reach..=centre + reach {
                if (0..len as isize).contains(&cell) {
                    mask[cell as usize] = true;
                }
            }
        }
    }
    voiced.then_some(mask)
}

/// Probability that a random on-partial cell outshines a random off-partial one (ROC AUC,
/// ties counted half). Invariant to any monotone remap of the column — so it judges the
/// *bank*, and no display mapping can move it within a frame.
fn frame_auc(values: &[f32], mask: &[bool]) -> Option<f32> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let (mut rank_sum, mut positives) = (0.0f64, 0usize);
    let mut start = 0;
    while start < order.len() {
        // A run of tied values shares the mean of its ranks (1-based).
        let mut end = start + 1;
        while end < order.len() && values[order[end]] == values[order[start]] {
            end += 1;
        }
        let mean_rank = (start + 1 + end) as f64 / 2.0;
        for &cell in &order[start..end] {
            if mask[cell] {
                rank_sum += mean_rank;
                positives += 1;
            }
        }
        start = end;
    }
    let negatives = values.len() - positives;
    if positives == 0 || negatives == 0 {
        return None;
    }
    let p = positives as f64;
    Some(((rank_sum - p * (p + 1.0) / 2.0) / (p * negatives as f64)) as f32)
}

/// A display mapping under test: raw column(s) → brightness in 0..=1, frame by frame.
/// Stateful ones carry their state in the `Vec<f32>` handed back in.
#[derive(Clone, Copy, Debug)]
enum Mapping {
    /// What ships: `normalize_bars` — divide by the frame max, raise to `gamma`.
    Shipped { gamma: f32 },
    /// A fixed dB window under the frame max: `range_db` below max is black.
    DbWindow { range_db: f32 },
    /// Per-cell noise floor that follows a quiet cell down at once and creeps up at
    /// `rise_db_per_s`; black = floor + `margin_db`, white = frame max.
    FloorTracker { rise_db_per_s: f32, margin_db: f32 },
    /// Per-frame background: the `percentile` of the column over ±`half_width` semitones;
    /// black = background + `margin_db`, white = frame max.
    LocalBackground {
        half_width: f32,
        percentile: f32,
        margin_db:  f32,
    },
    /// A `range_db` window under a white point that is the frame max **held** across
    /// frames, decaying at `decay_db_per_s` — so a quiet frame (a tail, a rest) stays dark
    /// instead of being divided up to full scale by its own small max.
    /// The window's 0..1 is then raised to `gamma` (1.0 = linear in dB) — the user's
    /// existing contrast slider, kept with the meaning it has today.
    PeakHold {
        range_db:       f32,
        decay_db_per_s: f32,
        gamma:          f32,
    },
    /// The shipped curve `(v / white)^gamma` with the white point **held** like
    /// `PeakHold`'s instead of being each frame's own max — the smallest change to what
    /// ships: the `gamma` slider keeps its meaning.
    HeldMax {
        gamma:          f32,
        decay_db_per_s: f32,
    },
    /// What ships now: `HeldWhite` + `map_display_levels` from the parent module, at the
    /// user's `gamma` — run through the same harness so the shipped code, not a copy of
    /// it, is what the verdict is about.
    Production { gamma: f32 },
}

const FLOOR_DB: f32 = -120.0;

fn to_db(column: &[f32], max: f32) -> Vec<f32> {
    // Relative to the frame max, floored: a gated-off cell is 0 and must not be −∞.
    column
        .iter()
        .map(|&m| (20.0 * (m / max).log10()).max(FLOOR_DB))
        .collect()
}

/// Map every column of a track; `None` for silent columns.
fn map_track(mapping: Mapping, columns: &[Vec<f32>]) -> Vec<Option<Vec<f32>>> {
    // Absolute per-cell floor (dB re full-scale magnitude 1) for `FloorTracker`.
    let mut floor: Vec<f32> = Vec::new();
    // Held white point (dB re magnitude 1) for `PeakHold`; `None` until the first frame.
    let mut held: Option<f32> = None;
    let mut production_white = super::HeldWhite::default();
    columns
        .iter()
        .map(|column| {
            let max = column.iter().fold(0.0f32, |m, &v| m.max(v));
            if let Mapping::Production { gamma } = mapping {
                // Fed every column, silent ones included — its decay must see their time.
                let white = production_white.update((max > 0.0).then_some(max), HOP_SECONDS);
                if max <= 0.0 {
                    return None;
                }
                let mut values = column.clone();
                super::map_display_levels(&mut values, white.unwrap(), gamma);
                return Some(values);
            }
            if max <= 0.0 {
                return None;
            }
            let db = to_db(column, max);
            Some(match mapping {
                Mapping::Production { .. } => unreachable!("mapped above, before the silence return"),
                Mapping::PeakHold {
                    range_db,
                    decay_db_per_s,
                    gamma,
                } => {
                    let max_db = 20.0 * max.log10();
                    let white = held.map_or(max_db, |h| max_db.max(h - decay_db_per_s * HOP_SECONDS));
                    held = Some(white);
                    // `db` is relative to this frame's max; shift it under the held white.
                    let shift = max_db - white;
                    db.iter()
                        .map(|&l| (1.0 + (l + shift) / range_db).clamp(0.0, 1.0).powf(gamma))
                        .collect()
                }
                Mapping::HeldMax {
                    gamma,
                    decay_db_per_s,
                } => {
                    let max_db = 20.0 * max.log10();
                    let white = held.map_or(max_db, |h| max_db.max(h - decay_db_per_s * HOP_SECONDS));
                    held = Some(white);
                    let white_linear = 10f32.powf(white / 20.0);
                    column
                        .iter()
                        .map(|&v| (v / white_linear).min(1.0).powf(gamma))
                        .collect()
                }
                Mapping::Shipped { gamma } => column.iter().map(|&v| (v / max).powf(gamma)).collect(),
                Mapping::DbWindow { range_db } => {
                    db.iter().map(|&l| (1.0 + l / range_db).clamp(0.0, 1.0)).collect()
                }
                Mapping::FloorTracker {
                    rise_db_per_s,
                    margin_db,
                } => {
                    let max_db = 20.0 * max.log10();
                    let absolute: Vec<f32> = db.iter().map(|&l| l + max_db).collect();
                    if floor.is_empty() {
                        floor = absolute.clone();
                    }
                    absolute
                        .iter()
                        .zip(floor.iter_mut())
                        .map(|(&l, f)| {
                            *f = l.min(*f + rise_db_per_s * HOP_SECONDS);
                            let black = *f + margin_db;
                            if black >= max_db {
                                0.0
                            } else {
                                ((l - black) / (max_db - black)).clamp(0.0, 1.0)
                            }
                        })
                        .collect()
                }
                Mapping::LocalBackground {
                    half_width,
                    percentile,
                    margin_db,
                } => {
                    let reach = (half_width * super::OUTPUT_BINS_PER_SEMITONE as f32) as usize;
                    let mut window = Vec::with_capacity(2 * reach + 1);
                    (0..db.len())
                        .map(|i| {
                            window.clear();
                            window.extend_from_slice(
                                &db[i.saturating_sub(reach)..(i + reach + 1).min(db.len())],
                            );
                            let k = ((window.len() - 1) as f32 * percentile) as usize;
                            let (_, background, _) = window.select_nth_unstable_by(k, f32::total_cmp);
                            let black = *background + margin_db;
                            if black >= 0.0 {
                                0.0
                            } else {
                                ((db[i] - black) / -black).clamp(0.0, 1.0)
                            }
                        })
                        .collect()
                }
            })
        })
        .collect()
}

/// How a mapping spends its light against the truth.
#[derive(Clone, Copy)]
struct Ink {
    /// Mean brightness of on-partial cells.
    on:               f32,
    /// Mean brightness of off-partial cells.
    off:              f32,
    /// Share of all light that lands on a partial, over every non-silent frame: a frame
    /// where no stem is voiced (a rest, a tail) has no partial to light, so all its light
    /// is off-partial — skipping it would flatter exactly the mappings that blow a tail up
    /// to full scale.
    precision:        f32,
    /// The same share over voiced frames only — what is left of the gain once rests are
    /// taken out of the question.
    voiced_precision: f32,
}

fn ink(mapped: &[Option<Vec<f32>>], masks: &[Option<Vec<bool>>]) -> Ink {
    let (mut on, mut on_n, mut off, mut off_n, mut voiced_off) = (0.0f64, 0usize, 0.0f64, 0usize, 0.0f64);
    for (values, mask) in mapped.iter().zip(masks) {
        let Some(values) = values else {
            continue;
        };
        for (index, &v) in values.iter().enumerate() {
            if mask.as_ref().is_some_and(|m| m[index]) {
                on += v as f64;
                on_n += 1;
            } else {
                off += v as f64;
                off_n += 1;
                if mask.is_some() {
                    voiced_off += v as f64;
                }
            }
        }
    }
    Ink {
        on:               (on / on_n as f64) as f32,
        off:              (off / off_n as f64) as f32,
        precision:        (on / (on + off)) as f32,
        voiced_precision: (on / (on + voiced_off)) as f32,
    }
}

#[test]
#[ignore = "a probe, not a check: needs datasets/MDB-stem-synth — see the module doc"]
fn truth_probe() {
    let songs = polyphonic_songs();
    assert!(!songs.is_empty(), "no song with ≥ 3 stems in the corpus");
    let lo_midi = ResonatorViewSettings::default().lo_midi();
    // Alpha axis only. No beta axis: probe 1 measured `(a, 1)` ≡ `(1, a)` to every printed
    // digit, and that is exact, not approximate — the bank's transfer function is
    // `αβ / ((1 − c·z⁻¹)(1 − d·z⁻¹))`, symmetric in swapping the two stages. The first
    // truth run also showed the diagonal `(s, s)` tracking `(s², 1)` closely, so it adds
    // no new direction worth the bank time.
    let configs: Vec<(f32, f32)> = SCALES.iter().map(|&a| (a, 1.0)).collect();
    let baseline = configs.iter().position(|&c| c == (1.0, 1.0)).unwrap();
    let mappings = [
        Mapping::Shipped { gamma: 0.72 },
        Mapping::Shipped { gamma: 1.0 },
        Mapping::Shipped { gamma: 1.6 },
        Mapping::DbWindow { range_db: 30.0 },
        Mapping::DbWindow { range_db: 36.0 },
        Mapping::Production { gamma: 0.72 },
        Mapping::Production { gamma: 1.6 },
        Mapping::PeakHold {
            range_db:       36.0,
            decay_db_per_s: 6.0,
            gamma:          1.0,
        },
        Mapping::PeakHold {
            range_db:       24.0,
            decay_db_per_s: 6.0,
            gamma:          0.72,
        },
        Mapping::PeakHold {
            range_db:       27.0,
            decay_db_per_s: 6.0,
            gamma:          0.72,
        },
        Mapping::PeakHold {
            range_db:       30.0,
            decay_db_per_s: 6.0,
            gamma:          0.72,
        },
        Mapping::PeakHold {
            range_db:       36.0,
            decay_db_per_s: 6.0,
            gamma:          0.72,
        },
        Mapping::PeakHold {
            range_db:       30.0,
            decay_db_per_s: 6.0,
            gamma:          1.6,
        },
        Mapping::HeldMax {
            gamma:          0.72,
            decay_db_per_s: 3.0,
        },
        Mapping::HeldMax {
            gamma:          0.72,
            decay_db_per_s: 6.0,
        },
        Mapping::HeldMax {
            gamma:          0.72,
            decay_db_per_s: 12.0,
        },
        Mapping::HeldMax {
            gamma:          0.5,
            decay_db_per_s: 6.0,
        },
        Mapping::FloorTracker {
            rise_db_per_s: 3.0,
            margin_db:     6.0,
        },
        Mapping::LocalBackground {
            half_width: 2.0,
            percentile: 0.5,
            margin_db:  6.0,
        },
    ];

    struct SongResult {
        name:  String,
        /// Per config: per-frame AUC, `None` where no stem is voiced.
        auc:   Vec<Vec<Option<f32>>>,
        /// Per config: per-block Rényi effective cells (probe 1's criterion — no truth).
        renyi: Vec<Vec<Option<f32>>>,
        /// Per config, per mapping: `ink`.
        ink:   Vec<Vec<Ink>>,
    }

    let results: Vec<SongResult> = std::thread::scope(|scope| {
        let handles: Vec<_> = songs
            .iter()
            .map(|song| {
                let (configs, mappings) = (&configs, &mappings);
                scope.spawn(move || {
                    let mut masks = None;
                    let (mut auc, mut renyi, mut inks) = (Vec::new(), Vec::new(), Vec::new());
                    for &(alpha, beta) in configs {
                        let cols = columns(song.sample_rate, &song.mix, alpha, beta);
                        let masks: &Vec<Option<Vec<bool>>> = masks.get_or_insert_with(|| {
                            (0..cols.len())
                                .map(|i| truth_mask(song, i, cols[0].len(), lo_midi))
                                .collect()
                        });
                        inks.push(
                            mappings
                                .iter()
                                .map(|&m| ink(&map_track(m, &cols), masks))
                                .collect(),
                        );
                        auc.push(
                            cols.iter()
                                .zip(masks)
                                .map(|(c, m)| frame_auc(c, m.as_ref()?))
                                .collect(),
                        );
                        renyi.push(sparsity(&cols).blocks);
                    }
                    SongResult {
                        name: song.name.clone(),
                        auc,
                        renyi,
                        ink: inks,
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let mean = |values: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = values.fold((0.0f64, 0usize), |(s, n), v| (s + v as f64, n + 1));
        (sum / n as f64) as f32
    };
    // Mean AUC of one block of frames, `None` when no frame in it is voiced.
    let block_auc = |frames: &[Option<f32>]| {
        let voiced: Vec<f32> = frames.iter().flatten().copied().collect();
        (!voiced.is_empty()).then(|| voiced.iter().sum::<f32>() / voiced.len() as f32)
    };

    println!("\n=== frame AUC: on-partial vs off-partial cells (raw bank, 1.0 = perfect) ===");
    print!("{:<34}", "song");
    for (alpha, _) in &configs {
        print!(" {:>7}", format!("a={alpha}"));
    }
    println!("   | adaptive per 0.5 s block: fixed1  by-Rényi  oracle");
    for r in &results {
        print!("{:<34}", r.name);
        for frames in &r.auc {
            print!(" {:>7.4}", mean(&mut frames.iter().flatten().copied()));
        }
        // Blocks where the song is voiced *and* the bank is not silent at alpha = 1.
        let blocks = r.renyi[baseline].len();
        let (mut fixed, mut by_renyi, mut oracle) = (Vec::new(), Vec::new(), Vec::new());
        for b in 0..blocks {
            let span = b * BLOCK_FRAMES..(b + 1) * BLOCK_FRAMES;
            let Some(at_unity) = block_auc(&r.auc[baseline][span.clone()]) else {
                continue;
            };
            if r.renyi[baseline][b].is_none() {
                continue;
            }
            let per_config: Vec<f32> = (0..configs.len())
                .map(|c| block_auc(&r.auc[c][span.clone()]).unwrap())
                .collect();
            let sparsest = (0..configs.len())
                .min_by(|&x, &y| r.renyi[x][b].unwrap().total_cmp(&r.renyi[y][b].unwrap()))
                .unwrap();
            fixed.push(at_unity);
            by_renyi.push(per_config[sparsest]);
            oracle.push(per_config.iter().copied().fold(0.0f32, f32::max));
        }
        println!(
            "   |   {:.4}    {:.4}   {:.4}",
            mean(&mut fixed.into_iter()),
            mean(&mut by_renyi.into_iter()),
            mean(&mut oracle.into_iter())
        );
    }

    println!("\n=== display mappings: on / off / precision (share of light on partials), mean of songs ===");
    for (c, (alpha, _)) in configs.iter().enumerate() {
        println!("-- alpha = {alpha}");
        for (index, mapping) in mappings.iter().enumerate() {
            let n = results.len() as f32;
            let avg =
                |field: fn(&Ink) -> f32| results.iter().map(|r| field(&r.ink[c][index])).sum::<f32>() / n;
            let (on, off) = (avg(|i| i.on), avg(|i| i.off));
            println!(
                "   {:<60} on {:.3}  off {:.4}  on/off {:>5.1}  precision {:.3}  voiced {:.3}",
                format!("{mapping:?}"),
                on,
                off,
                on / off,
                avg(|i| i.precision),
                avg(|i| i.voiced_precision)
            );
        }
    }
}
