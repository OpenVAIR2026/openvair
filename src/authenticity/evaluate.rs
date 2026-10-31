//! Offline evaluation of baselines and the drift check for imported batches.
//!
//! Evaluation repeats random train/test splits and reports the mean and the
//! 10th percentile across splits; a single split was shown in the feasibility
//! spike to swing by more than ten points.
use super::fingerprint::{Counts, jsd, mean_loglik};
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use serde::Serialize;
use std::collections::BTreeMap;

pub const KS: [usize; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
const TARGET_ACCURACY: f64 = 0.95;
/// Below this many valid samples a model is not evaluated.
pub const MIN_EVAL_SAMPLES: usize = 30;

pub struct ModelData {
    pub model: String,
    pub family: String,
    pub samples: Vec<Vec<u16>>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stat {
    pub mean: f64,
    pub p10: f64,
}

#[derive(Debug, Serialize)]
pub struct PairResult {
    pub model: String,
    pub family: String,
    pub same_family: bool,
    /// k -> accuracy of telling the target and this model apart in both directions.
    pub accuracy: BTreeMap<usize, Stat>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Needed {
    /// Whether any candidate of this kind (cross-family or sibling) was compared.
    pub compared: bool,
    /// Fewest detection samples whose mean accuracy reaches 95% for every pair.
    pub mean: Option<usize>,
    /// The same using the 10th-percentile split, a conservative figure.
    pub p10: Option<usize>,
    /// Largest detection sample count the held-out samples could test for
    /// every pair. Beyond it "not reached" is unknown, not a failure.
    pub evaluated_up_to: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct Evaluation {
    pub model: String,
    pub samples: usize,
    pub splits: usize,
    pub pairs: Vec<PairResult>,
    pub cross_family: Needed,
    pub same_family: Needed,
    /// Models skipped for having too few samples.
    pub skipped: Vec<String>,
    pub recommendation: Option<String>,
}

pub fn evaluate(
    target: &str,
    models: &[ModelData],
    range_max: u16,
    splits: usize,
    trials: usize,
    seed: u64,
) -> Option<Evaluation> {
    let usable: Vec<&ModelData> = models
        .iter()
        .filter(|m| m.samples.len() >= MIN_EVAL_SAMPLES)
        .collect();
    let skipped = models
        .iter()
        .filter(|m| m.samples.len() < MIN_EVAL_SAMPLES)
        .map(|m| m.model.clone())
        .collect();
    let target_index = usable.iter().position(|m| m.model == target)?;
    let target_family = usable[target_index].family.clone();
    let mut rng = StdRng::seed_from_u64(seed);
    // pair index -> k -> per-split accuracies
    let mut per_split: Vec<BTreeMap<usize, Vec<f64>>> = vec![BTreeMap::new(); usable.len()];
    for _ in 0..splits {
        let mut log_probs = Vec::with_capacity(usable.len());
        let mut tests = Vec::with_capacity(usable.len());
        for model in &usable {
            let mut shuffled: Vec<&Vec<u16>> = model.samples.iter().collect();
            shuffled.shuffle(&mut rng);
            let cut = (shuffled.len() * 3 / 10).max(10);
            let (test, train) = shuffled.split_at(cut);
            log_probs.push(
                Counts::from_samples(range_max, train.iter().map(|s| s.as_slice())).log_probs(),
            );
            tests.push(test.to_vec());
        }
        // Score each test sample under the target and its own model only.
        let target_lp = &log_probs[target_index];
        for (other, model) in usable.iter().enumerate() {
            if other == target_index {
                continue;
            }
            let _ = model;
            let other_lp = &log_probs[other];
            // Positive margin: the sample's true model scores higher.
            let target_margins: Vec<f64> = tests[target_index]
                .iter()
                .map(|s| mean_loglik(s, target_lp) - mean_loglik(s, other_lp))
                .collect();
            let other_margins: Vec<f64> = tests[other]
                .iter()
                .map(|s| mean_loglik(s, other_lp) - mean_loglik(s, target_lp))
                .collect();
            for &k in &KS {
                if k > target_margins.len() || k > other_margins.len() {
                    continue;
                }
                let a = accuracy(&target_margins, k, trials, &mut rng);
                let b = accuracy(&other_margins, k, trials, &mut rng);
                per_split[other].entry(k).or_default().push(a.min(b));
            }
        }
    }
    let mut pairs = Vec::new();
    for (index, model) in usable.iter().enumerate() {
        if index == target_index {
            continue;
        }
        let accuracy = per_split[index]
            .iter()
            .map(|(&k, values)| (k, summarize(values)))
            .collect();
        pairs.push(PairResult {
            model: model.model.clone(),
            family: model.family.clone(),
            same_family: model.family == target_family,
            accuracy,
        });
    }
    let cross_family = needed(pairs.iter().filter(|p| !p.same_family));
    let same_family = needed(pairs.iter().filter(|p| p.same_family));
    let has_siblings = pairs.iter().any(|p| p.same_family);
    let unknown_beyond = |n: &Needed| n.p10.is_none() && n.evaluated_up_to.is_none_or(|k| k < 32);
    let recommendation = if pairs.is_empty() {
        Some("还没有其他启用的基准可比较，启用更多模型后会自动重新评估".into())
    } else if has_siblings && unknown_beyond(&same_family) {
        Some(format!(
            "基准样本只够评估到 {} 个检测样本，同家族型号在此范围内未达到 95%；补采基准后才能评估更多检测样本的效果",
            same_family.evaluated_up_to.unwrap_or(0)
        ))
    } else if has_siblings && same_family.p10.is_none_or(|k| k > 32) {
        Some("同家族型号用 32 个检测样本仍达不到 95%，建议提高目标样本数并增量补采".into())
    } else if cross_family.p10.is_none_or(|k| k > 16) && pairs.iter().any(|p| !p.same_family) {
        Some("跨厂商替换用 16 个检测样本仍达不到 95%，建议增量补采".into())
    } else {
        None
    };
    Some(Evaluation {
        model: target.to_owned(),
        samples: usable[target_index].samples.len(),
        splits,
        pairs,
        cross_family,
        same_family,
        skipped,
        recommendation,
    })
}

/// Share of random k-sample draws whose summed margin is positive.
fn accuracy(margins: &[f64], k: usize, trials: usize, rng: &mut StdRng) -> f64 {
    let mut ok = 0;
    for _ in 0..trials {
        let total: f64 = margins.choose_multiple(rng, k).sum();
        ok += usize::from(total > 0.0);
    }
    ok as f64 / trials as f64
}

fn summarize(values: &[f64]) -> Stat {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Stat {
        mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p10: sorted[sorted.len() / 10],
    }
}

fn needed<'a>(pairs: impl Iterator<Item = &'a PairResult> + Clone) -> Needed {
    let first = |pick: fn(&Stat) -> f64| {
        KS.iter().copied().find(|k| {
            let mut any = false;
            let all = pairs.clone().all(|p| {
                any = true;
                p.accuracy
                    .get(k)
                    .is_some_and(|stat| pick(stat) >= TARGET_ACCURACY)
            });
            any && all
        })
    };
    let evaluated_up_to = KS
        .iter()
        .copied()
        .take_while(|k| pairs.clone().all(|p| p.accuracy.contains_key(k)))
        .last()
        .filter(|_| pairs.clone().next().is_some());
    Needed {
        compared: pairs.clone().next().is_some(),
        mean: first(|s| s.mean),
        p10: first(|s| s.p10),
        evaluated_up_to,
    }
}

#[derive(Debug, Serialize)]
pub struct Drift {
    pub checked: bool,
    pub jsd: f64,
    pub p_value: f64,
    pub flagged: bool,
}

/// Permutation test: is the batch's value distribution further from the
/// existing samples than random splits of the pooled samples would be?
pub fn drift(
    existing: &[Vec<u16>],
    batch: &[Vec<u16>],
    range_max: u16,
    permutations: usize,
    seed: u64,
) -> Drift {
    if existing.len() < 20 || batch.len() < 10 {
        return Drift {
            checked: false,
            jsd: 0.0,
            p_value: 1.0,
            flagged: false,
        };
    }
    let observed = jsd(
        &Counts::from_samples(range_max, existing.iter().map(Vec::as_slice)),
        &Counts::from_samples(range_max, batch.iter().map(Vec::as_slice)),
    );
    let pooled: Vec<&Vec<u16>> = existing.iter().chain(batch).collect();
    let total = Counts::from_samples(range_max, pooled.iter().map(|s| s.as_slice()));
    let mut rng = StdRng::seed_from_u64(seed);
    let mut indices: Vec<usize> = (0..pooled.len()).collect();
    let mut at_least = 0;
    for _ in 0..permutations {
        indices.shuffle(&mut rng);
        let part = Counts::from_samples(
            range_max,
            indices[..batch.len()].iter().map(|&i| pooled[i].as_slice()),
        );
        let rest = Counts {
            range_max,
            counts: total
                .counts
                .iter()
                .zip(&part.counts)
                .map(|(t, p)| t - p)
                .collect(),
            samples: total.samples - part.samples,
        };
        at_least += usize::from(jsd(&rest, &part) >= observed);
    }
    let p_value = (at_least + 1) as f64 / (permutations + 1) as f64;
    Drift {
        checked: true,
        jsd: observed,
        p_value,
        flagged: p_value < 0.01,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    fn synthetic(seed: u64, favourites: &[u16], count: usize) -> Vec<Vec<u16>> {
        let mut table: Vec<u16> = (1..=355).collect();
        for &f in favourites {
            table.extend(std::iter::repeat_n(f, 8));
        }
        let mut rng = StdRng::seed_from_u64(seed);
        (0..count)
            .map(|_| {
                (0..200)
                    .map(|_| table[rng.gen_range(0..table.len())])
                    .collect()
            })
            .collect()
    }

    #[test]
    fn distinct_models_are_separable_with_few_samples() {
        let models = vec![
            ModelData {
                model: "a".into(),
                family: "x".into(),
                samples: synthetic(1, &[7, 42, 99, 200], 120),
            },
            ModelData {
                model: "b".into(),
                family: "y".into(),
                samples: synthetic(2, &[13, 77, 150, 300], 120),
            },
            ModelData {
                model: "tiny".into(),
                family: "y".into(),
                samples: synthetic(3, &[1], 5),
            },
        ];
        let result = evaluate("a", &models, 355, 5, 50, 9).unwrap();
        assert_eq!(result.pairs.len(), 1);
        assert_eq!(result.skipped, vec!["tiny".to_owned()]);
        assert!(result.cross_family.mean.is_some_and(|k| k <= 4));
        assert!(result.cross_family.compared);
        // 120 samples hold out 36 for testing: 32 is the largest testable count.
        assert_eq!(result.cross_family.evaluated_up_to, Some(32));
        assert_eq!(result.same_family.evaluated_up_to, None);
        assert!(!result.same_family.compared);
        assert!(result.same_family.mean.is_none());
        assert!(evaluate("missing", &models, 355, 5, 50, 9).is_none());
    }

    #[test]
    fn drift_flags_a_different_distribution_only() {
        let existing = synthetic(1, &[7, 42, 99, 200], 100);
        let same = synthetic(5, &[7, 42, 99, 200], 40);
        let changed = synthetic(6, &[13, 77, 150, 300], 40);
        assert!(!drift(&existing, &same, 355, 200, 1).flagged);
        assert!(drift(&existing, &changed, 355, 200, 1).flagged);
        assert!(!drift(&existing[..5], &changed, 355, 200, 1).checked);
    }
}
