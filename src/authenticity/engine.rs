//! Turns fingerprint and tokenizer evidence into an identity verdict.
//!
//! Each valid sample yields one per-candidate score (its mean log-likelihood
//! under that candidate's baseline). For the claimed model `X` and every
//! alternative `Y`, the per-sample differences `score(X) - score(Y)` are
//! roughly independent across requests, so a one-sided t statistic tells how
//! strongly the samples favour one side. Calling a mismatch needs stronger
//! evidence than calling a match: a false accusation costs more than a miss.
use super::fingerprint::mean_loglik;
use serde::Serialize;

/// Fewest valid fingerprint samples before any verdict other than insufficient.
pub const MIN_SAMPLES: usize = 8;
/// One-sided 95%: enough to call a match or a suspected mismatch.
const Z_MATCH: f64 = 1.645;
/// One-sided 99.5%: needed (with corroboration) to confirm a mismatch.
const Z_STRICT: f64 = 2.576;
/// Same-family substitutions have no tokenizer corroboration, so they need more samples.
const SIBLING_CONFIRM_SAMPLES: usize = 32;
const CROSS_CONFIRM_SAMPLES: usize = 16;

pub struct Candidate {
    pub model: String,
    pub family: String,
    pub log_probs: Vec<f64>,
    /// Baseline token-count difference between the long and short token probes.
    pub token_diff: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Match,
    FamilyOnly,
    Suspected,
    Confirmed,
    Insufficient,
    Ambiguous,
    Unsupported,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Match => "match",
            Verdict::FamilyOnly => "family_only",
            Verdict::Suspected => "suspected",
            Verdict::Confirmed => "confirmed",
            Verdict::Insufficient => "insufficient",
            Verdict::Ambiguous => "ambiguous",
            Verdict::Unsupported => "unsupported",
        }
    }

    /// A deep check may stop once the verdict cannot reasonably change.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            Verdict::Match | Verdict::Confirmed | Verdict::Unsupported
        )
    }
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub model: String,
    pub family: String,
    pub same_family: bool,
    /// Mean of score(claimed) - score(model); negative favours this model.
    pub mean_diff: f64,
    pub z: f64,
    /// Whether the observed tokenizer difference fits this model's baseline.
    pub token_consistent: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Tokenizer {
    pub observed_diff: f64,
    pub claimed_diff: Option<f64>,
    pub consistent: Option<bool>,
    /// Candidate families whose baseline matches the observed difference.
    pub matching_families: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Judgement {
    pub verdict: Verdict,
    pub claimed: String,
    pub claimed_family: Option<String>,
    pub valid_samples: usize,
    /// Candidate with the highest total score.
    pub best_model: Option<String>,
    pub comparisons: Vec<Comparison>,
    pub tokenizer: Option<Tokenizer>,
    /// Plain-language reasons for the report.
    pub reasons: Vec<String>,
}

fn token_consistent(observed: f64, baseline: f64) -> bool {
    (observed - baseline).abs() <= (baseline.abs() * 0.02).max(1.0)
}

pub fn judge(
    claimed: &str,
    candidates: &[Candidate],
    samples: &[Vec<u16>],
    token_diff: Option<f64>,
) -> Judgement {
    let mut out = Judgement {
        verdict: Verdict::Insufficient,
        claimed: claimed.to_owned(),
        claimed_family: None,
        valid_samples: samples.len(),
        best_model: None,
        comparisons: Vec::new(),
        tokenizer: None,
        reasons: Vec::new(),
    };
    let Some(own) = candidates.iter().find(|c| c.model == claimed) else {
        out.verdict = Verdict::Unsupported;
        out.reasons.push(format!("{claimed} 还没有启用的基准"));
        return out;
    };
    out.claimed_family = Some(own.family.clone());

    // Per-sample scores under every candidate.
    let scores: Vec<Vec<f64>> = candidates
        .iter()
        .map(|c| {
            samples
                .iter()
                .map(|s| mean_loglik(s, &c.log_probs))
                .collect()
        })
        .collect();
    let own_index = candidates.iter().position(|c| c.model == claimed).unwrap();
    for (index, candidate) in candidates.iter().enumerate() {
        if index == own_index {
            continue;
        }
        let diffs: Vec<f64> = scores[own_index]
            .iter()
            .zip(&scores[index])
            .map(|(a, b)| a - b)
            .collect();
        let (mean_diff, z) = t_statistic(&diffs);
        out.comparisons.push(Comparison {
            model: candidate.model.clone(),
            family: candidate.family.clone(),
            same_family: candidate.family == own.family,
            mean_diff,
            z,
            token_consistent: token_diff
                .zip(candidate.token_diff)
                .map(|(observed, baseline)| token_consistent(observed, baseline)),
        });
    }
    out.comparisons.sort_by(|a, b| a.z.total_cmp(&b.z));
    // The closest model is the one the verdict weighs: the candidate the
    // samples lean to most consistently, or the claim when none is favoured.
    // A larger but noisier average gap must not name a different model.
    if !samples.is_empty() {
        out.best_model = Some(
            out.comparisons
                .first()
                .filter(|c| c.z < 0.0)
                .map_or_else(|| claimed.to_owned(), |c| c.model.clone()),
        );
    }

    if let Some(observed) = token_diff {
        let matching_families = {
            let mut families: Vec<String> = candidates
                .iter()
                .filter(|c| c.token_diff.is_some_and(|d| token_consistent(observed, d)))
                .map(|c| c.family.clone())
                .collect();
            families.sort();
            families.dedup();
            families
        };
        out.tokenizer = Some(Tokenizer {
            observed_diff: observed,
            claimed_diff: own.token_diff,
            consistent: own.token_diff.map(|d| token_consistent(observed, d)),
            matching_families,
        });
    }
    let tokenizer_consistent = out.tokenizer.as_ref().and_then(|t| t.consistent);

    if samples.len() < MIN_SAMPLES {
        out.reasons.push(format!(
            "有效样本 {} 个，至少需要 {MIN_SAMPLES} 个",
            samples.len()
        ));
        if tokenizer_consistent == Some(false) {
            out.reasons
                .push("Tokenizer 计数与申报模型的基准不一致".into());
        }
        return out;
    }

    let strongest = out.comparisons.first();
    let beats_all = out.comparisons.iter().all(|c| c.z > Z_MATCH);
    let beats_other_families = out
        .comparisons
        .iter()
        .filter(|c| !c.same_family)
        .all(|c| c.z > Z_MATCH);
    let favoured = strongest.filter(|c| c.z < -Z_MATCH);

    out.verdict = match favoured {
        // The tokenizer fits the claimed model and rules out the favoured one.
        Some(alt)
            if !alt.same_family
                && tokenizer_consistent == Some(true)
                && alt.token_consistent == Some(false)
                && alt.z < -Z_STRICT =>
        {
            out.reasons.push(format!(
                "数字分布明显更像 {}，但 Tokenizer 计数与申报模型一致，两类证据冲突",
                alt.model
            ));
            Verdict::Ambiguous
        }
        Some(alt) if alt.z < -Z_STRICT => {
            let corroborated = if alt.same_family {
                samples.len() >= SIBLING_CONFIRM_SAMPLES
            } else {
                samples.len() >= CROSS_CONFIRM_SAMPLES && tokenizer_consistent == Some(false)
            };
            out.reasons.push(format!(
                "数字分布明显更像 {}（z = {:.1}）",
                alt.model, alt.z
            ));
            if corroborated {
                if !alt.same_family {
                    out.reasons
                        .push("Tokenizer 计数也与申报模型的基准不一致".into());
                }
                Verdict::Confirmed
            } else {
                out.reasons.push(if alt.same_family {
                    format!("同家族替换需要至少 {SIBLING_CONFIRM_SAMPLES} 个有效样本才能确认")
                } else {
                    "缺少 Tokenizer 的同向证据，暂不确认".into()
                });
                Verdict::Suspected
            }
        }
        Some(alt) => {
            out.reasons.push(format!(
                "数字分布更像 {}（z = {:.1}），证据尚不充分",
                alt.model, alt.z
            ));
            Verdict::Suspected
        }
        None if beats_all && tokenizer_consistent == Some(false) => {
            out.reasons
                .push("数字分布与申报模型一致，但 Tokenizer 计数不一致，两类证据冲突".into());
            Verdict::Ambiguous
        }
        None if beats_all => {
            out.reasons
                .push("数字分布与申报模型的基准一致，并明显区别于其他候选模型".into());
            Verdict::Match
        }
        None if beats_other_families && tokenizer_consistent != Some(false) => {
            out.reasons
                .push("可以确认属于申报模型的厂商，但与同家族型号尚未区分开".into());
            Verdict::FamilyOnly
        }
        None => {
            out.reasons
                .push("现有样本还不足以区分申报模型与其他候选模型".into());
            Verdict::Insufficient
        }
    };
    out
}

/// Mean and one-sample t statistic against zero.
fn t_statistic(diffs: &[f64]) -> (f64, f64) {
    let n = diffs.len() as f64;
    if diffs.is_empty() {
        return (0.0, 0.0);
    }
    let mean = diffs.iter().sum::<f64>() / n;
    if diffs.len() < 2 {
        return (mean, 0.0);
    }
    let variance = diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let se = (variance.max(1e-12) / n).sqrt();
    (mean, mean / se)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authenticity::fingerprint::Counts;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    /// A synthetic model that prefers `favourites` over other values.
    fn sampler(favourites: &[u16], weight: u32) -> impl Fn(&mut StdRng) -> Vec<u16> {
        let mut table: Vec<u16> = (1..=355).collect();
        for &f in favourites {
            for _ in 0..weight {
                table.push(f);
            }
        }
        move |rng: &mut StdRng| {
            (0..200)
                .map(|_| table[rng.gen_range(0..table.len())])
                .collect()
        }
    }

    fn candidate(model: &str, family: &str, samples: &[Vec<u16>], token: f64) -> Candidate {
        Candidate {
            model: model.into(),
            family: family.into(),
            log_probs: Counts::from_samples(355, samples.iter().map(Vec::as_slice)).log_probs(),
            token_diff: Some(token),
        }
    }

    type Sampler = Box<dyn Fn(&mut StdRng) -> Vec<u16>>;

    struct World {
        rng: StdRng,
        big: Sampler,
        small: Sampler,
        other: Sampler,
        candidates: Vec<Candidate>,
    }

    fn world() -> World {
        let mut rng = StdRng::seed_from_u64(7);
        let big: Sampler = Box::new(sampler(&[7, 42, 73, 137, 211, 300], 6));
        let small: Sampler = Box::new(sampler(&[7, 42, 73, 150, 222, 333], 6));
        let other: Sampler = Box::new(sampler(&[12, 88, 199, 250, 301, 350], 6));
        let base = |f: &dyn Fn(&mut StdRng) -> Vec<u16>, rng: &mut StdRng| {
            (0..200).map(|_| f(rng)).collect::<Vec<_>>()
        };
        let candidates = vec![
            candidate("big", "acme", &base(&*big, &mut rng), 79.0),
            candidate("small", "acme", &base(&*small, &mut rng), 79.0),
            candidate("other", "zeta", &base(&*other, &mut rng), 83.0),
        ];
        World {
            rng,
            big,
            small,
            other,
            candidates,
        }
    }

    fn draw(f: &dyn Fn(&mut StdRng) -> Vec<u16>, rng: &mut StdRng, k: usize) -> Vec<Vec<u16>> {
        (0..k).map(|_| f(rng)).collect()
    }

    #[test]
    fn genuine_model_matches() {
        let mut w = world();
        let samples = draw(&*w.big, &mut w.rng, 32);
        let j = judge("big", &w.candidates, &samples, Some(79.0));
        assert_eq!(j.verdict, Verdict::Match, "{:?}", j.reasons);
        assert_eq!(j.best_model.as_deref(), Some("big"));
    }

    #[test]
    fn closest_model_is_the_one_the_verdict_names() {
        let mut w = world();
        let samples = draw(&*w.other, &mut w.rng, 16);
        let j = judge("big", &w.candidates, &samples, Some(83.0));
        assert_eq!(
            j.best_model.as_deref(),
            Some(j.comparisons[0].model.as_str())
        );
        assert!(j.comparisons[0].z < 0.0);
    }

    #[test]
    fn cross_vendor_substitution_is_confirmed_with_tokenizer() {
        let mut w = world();
        let samples = draw(&*w.other, &mut w.rng, 16);
        assert_eq!(
            judge("big", &w.candidates, &samples, Some(83.0)).verdict,
            Verdict::Confirmed
        );
        // Without tokenizer corroboration it is only suspected.
        assert_eq!(
            judge("big", &w.candidates, &samples, None).verdict,
            Verdict::Suspected
        );
        // Conflicting tokenizer evidence is ambiguous, never a confirmation.
        assert_eq!(
            judge("big", &w.candidates, &samples, Some(79.0)).verdict,
            Verdict::Ambiguous
        );
    }

    #[test]
    fn sibling_substitution_needs_more_samples_to_confirm() {
        let mut w = world();
        let few = draw(&*w.small, &mut w.rng, 16);
        let j = judge("big", &w.candidates, &few, Some(79.0));
        assert!(matches!(j.verdict, Verdict::Suspected), "{:?}", j.verdict);
        let many = draw(&*w.small, &mut w.rng, 40);
        assert_eq!(
            judge("big", &w.candidates, &many, Some(79.0)).verdict,
            Verdict::Confirmed
        );
    }

    #[test]
    fn few_samples_or_missing_baseline_never_decide() {
        let mut w = world();
        let samples = draw(&*w.other, &mut w.rng, MIN_SAMPLES - 1);
        assert_eq!(
            judge("big", &w.candidates, &samples, Some(83.0)).verdict,
            Verdict::Insufficient
        );
        assert_eq!(
            judge("unknown", &w.candidates, &samples, None).verdict,
            Verdict::Unsupported
        );
    }

    #[test]
    fn t_statistic_handles_degenerate_input() {
        assert_eq!(t_statistic(&[]), (0.0, 0.0));
        assert_eq!(t_statistic(&[1.0]).1, 0.0);
        assert!(t_statistic(&[1.0, 1.0, 1.0]).1 > 1e3);
    }
}
