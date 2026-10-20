//! H4: proximity of octave errors to changes in reference f0, without invented note labels.

use super::{
    Annotation,
    Verdict,
};

/// Distance buckets in seconds, chosen around the 16 ms hop and 46 ms FFT window.
const DISTANCE_EDGES: [f32; 4] = [0.008, 0.024, 0.048, 0.096];
const BUCKETS: usize = 6; // four near buckets, >96 ms, no reference change in the track

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DistanceBucket(usize);

pub(super) struct ReferenceChanges {
    /// Silence edges or >=50 cents in one native annotation step (abrupt changes).
    abrupt: Vec<f32>,
    /// Silence edges or >=5 cents in one native step (H1's non-standing truth).
    moving: Vec<f32>,
}

impl ReferenceChanges {
    pub(super) fn new(annotation: &Annotation) -> Self {
        let mut changes = Self {
            abrupt: Vec::new(),
            moving: Vec::new(),
        };
        for (times, pitches) in annotation.times.windows(2).zip(annotation.midi.windows(2)) {
            let cents = match (pitches[0], pitches[1]) {
                (Some(a), Some(b)) => (b - a).abs() * 100.0,
                (None, None) => continue,
                _ => f32::INFINITY,
            };
            // `Annotation::at` changes nearest neighbour at this midpoint.
            let seconds = (times[0] + times[1]) * 0.5;
            if cents >= super::RPA_TOLERANCE_CENTS {
                changes.abrupt.push(seconds);
            }
            if cents >= super::NOTE_STANDS_CENTS {
                changes.moving.push(seconds);
            }
        }
        changes
    }
}

fn bucket(changes: &[f32], seconds: f32) -> DistanceBucket {
    if changes.is_empty() {
        return DistanceBucket(BUCKETS - 1);
    }
    let after = changes.partition_point(|&t| t < seconds);
    let distance = changes[after.saturating_sub(1)..(after + 1).min(changes.len())]
        .iter()
        .map(|t| (seconds - t).abs())
        .fold(f32::INFINITY, f32::min);
    DistanceBucket(DISTANCE_EDGES.partition_point(|&edge| distance > edge))
}

/// Counts the background exposure too: clustering of errors alone is insufficient when
/// almost all voiced audio is near a change. Down/up include all whole-octave multiples.
#[derive(Default, Clone)]
pub(super) struct Diagnostics {
    // [change definition][distance][voiced, octave down, octave up]
    counts: [[[u32; 3]; BUCKETS]; 2],
}

impl Diagnostics {
    pub(super) fn follow(
        &mut self,
        changes: &ReferenceChanges,
        seconds: f32,
        verdict: Verdict,
        truth: Option<f32>,
        estimate: Option<f32>,
    ) {
        let Some(truth) = truth else { return };
        for (counts, marks) in self.counts.iter_mut().zip([&changes.abrupt, &changes.moving]) {
            let row = &mut counts[bucket(marks, seconds).0];
            row[0] += 1;
            if verdict == Verdict::Octave {
                row[if estimate.unwrap() < truth { 1 } else { 2 }] += 1;
            }
        }
    }

    pub(super) fn merge(&mut self, other: &Self) {
        for (a, b) in self
            .counts
            .iter_mut()
            .flatten()
            .flatten()
            .zip(other.counts.iter().flatten().flatten())
        {
            *a += b;
        }
    }

    pub(super) fn report(&self, lag_ms: f32) {
        if self.counts[0].iter().map(|row| row[0]).sum::<u32>() == 0 {
            return;
        }
        for (name, counts) in ["abrupt >=50c or voicing edge", "moving >=5c or voicing edge"]
            .iter()
            .zip(&self.counts)
        {
            println!("  H4 reference changes: {name}, lag {lag_ms:.0} ms; distance from lagged truth time");
            println!("    distance ms       voiced  octave_down  octave_up  octave/voiced %");
            for (label, row) in ["0..8", "8..24", "24..48", "48..96", ">96", "no change"]
                .iter()
                .zip(counts)
            {
                println!(
                    "    {label:12} {:10} {:12} {:10} {:16.4}",
                    row[0],
                    row[1],
                    row[2],
                    100.0 * (row[1] + row[2]) as f32 / row[0].max(1) as f32
                );
            }
        }
    }
}

#[test]
fn reference_changes_keep_native_grid_midpoints_and_separate_motion() {
    let annotation = Annotation {
        times: vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5],
        midi:  vec![None, Some(60.0), Some(60.125), Some(60.125), Some(72.0), None],
    };
    let changes = ReferenceChanges::new(&annotation);
    assert_eq!(changes.abrupt.len(), 3);
    assert_eq!(changes.moving.len(), 4);
    assert!((changes.moving[1] - 0.15).abs() < 1e-6);
    assert_eq!(bucket(&changes.abrupt, 0.2), DistanceBucket(4));
    assert_eq!(bucket(&changes.moving, 0.2), DistanceBucket(3));
    assert_eq!(bucket(&[], 0.2), DistanceBucket(5));
    assert_eq!(bucket(&[0.5], 0.5), DistanceBucket(0));
    assert_eq!(bucket(&[0.5], 0.52), DistanceBucket(1));
    assert_eq!(bucket(&[0.5], 0.54), DistanceBucket(2));
    assert_eq!(bucket(&[0.5], 0.57), DistanceBucket(3));
    let mut tally = super::Tally::default();
    let scoring = super::Scoring {
        mask:        super::PitchMask::unbounded(),
        lag_seconds: 0.15,
    };
    // Estimate time .5 reads truth at .35, exactly the abrupt-change midpoint.
    super::score_frame(&mut tally, 0.5, &annotation, &scoring, Some((48.0, 1.0)));
    let prev = tally.calm.prev.unwrap();
    let mut diagnostics = Diagnostics::default();
    diagnostics.follow(
        &changes,
        prev.seconds - scoring.lag_seconds,
        prev.verdict,
        prev.truth,
        prev.estimate,
    );
    let mut merged = diagnostics.clone();
    merged.merge(&diagnostics);
    assert_eq!(merged.counts[0][0], [2, 2, 0]);
    assert_eq!(
        merged.counts[0].iter().map(|r| r[1] + r[2]).sum::<u32>(),
        2 * tally.octave
    );
}
