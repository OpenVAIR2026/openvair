//! Integer-sequence fingerprints: parsing answers, validating samples and
//! scoring them against a model's value distribution.

/// Smoothing added to every value bin so unseen values keep a finite score.
const ALPHA: f64 = 0.5;

/// Every integer in an answer, unfiltered. Values too large for `i64` are skipped.
pub fn parse_ints(text: &str) -> Vec<i64> {
    let mut values = Vec::new();
    let mut current: Option<i64> = None;
    for c in text.chars() {
        if let Some(digit) = c.to_digit(10) {
            current = Some(
                current
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(i64::from(digit)),
            );
        } else if let Some(value) = current.take() {
            values.push(value);
        }
    }
    values.extend(current);
    values
}

/// Cleans raw integers into a sample, or returns why it is unusable.
///
/// Stray out-of-range values are dropped unless they exceed 5%; answers that
/// keep writing past `n` are truncated. Without reasoning, models often stop
/// early, and those values are still a valid fingerprint, so only answers
/// under half of `n` (or under 60 values) are rejected.
pub fn validate(values: &[i64], n: usize, range_max: u16) -> Result<Vec<u16>, &'static str> {
    if values.is_empty() {
        return Err("no_numbers");
    }
    let clean: Vec<u16> = values
        .iter()
        .filter(|&&value| (1..=i64::from(range_max)).contains(&value))
        .map(|&value| value as u16)
        .collect();
    if values.len() - clean.len() > (values.len() / 20).max(1) {
        return Err("out_of_range");
    }
    if (clean.len() as f64) < (n as f64 * 0.5).max(60.0) {
        return Err("too_few");
    }
    Ok(clean.into_iter().take(n).collect())
}

/// Value counts of a model's baseline samples, indexed by value (0 unused).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Counts {
    pub range_max: u16,
    pub counts: Vec<u64>,
    pub samples: u64,
}

impl Counts {
    pub fn new(range_max: u16) -> Self {
        Self {
            range_max,
            counts: vec![0; usize::from(range_max) + 1],
            samples: 0,
        }
    }

    pub fn add(&mut self, sample: &[u16]) {
        for &value in sample {
            if value >= 1 && value <= self.range_max {
                self.counts[usize::from(value)] += 1;
            }
        }
        self.samples += 1;
    }

    pub fn from_samples<'a>(range_max: u16, samples: impl IntoIterator<Item = &'a [u16]>) -> Self {
        let mut counts = Self::new(range_max);
        for sample in samples {
            counts.add(sample);
        }
        counts
    }

    /// Smoothed natural-log probabilities, indexed by value.
    pub fn log_probs(&self) -> Vec<f64> {
        let total: f64 =
            self.counts.iter().skip(1).sum::<u64>() as f64 + ALPHA * f64::from(self.range_max);
        let mut out = vec![f64::NEG_INFINITY; self.counts.len()];
        for (value, &count) in self.counts.iter().enumerate().skip(1) {
            out[value] = ((count as f64 + ALPHA) / total).ln();
        }
        out
    }
}

/// Mean per-value log-likelihood of a sample. Averaging makes long and short
/// answers count as one observation each.
pub fn mean_loglik(sample: &[u16], log_probs: &[f64]) -> f64 {
    if sample.is_empty() {
        return f64::NEG_INFINITY;
    }
    sample
        .iter()
        .map(|&value| log_probs[usize::from(value)])
        .sum::<f64>()
        / sample.len() as f64
}

/// Jensen-Shannon divergence between two count vectors of the same range.
pub fn jsd(a: &Counts, b: &Counts) -> f64 {
    let p = normalized(a);
    let q = normalized(b);
    let mut divergence = 0.0;
    for (x, y) in p.iter().zip(&q) {
        let m = (x + y) / 2.0;
        if *x > 0.0 {
            divergence += 0.5 * x * (x / m).ln();
        }
        if *y > 0.0 {
            divergence += 0.5 * y * (y / m).ln();
        }
    }
    divergence
}

fn normalized(counts: &Counts) -> Vec<f64> {
    let total: f64 =
        counts.counts.iter().skip(1).sum::<u64>() as f64 + ALPHA * f64::from(counts.range_max);
    counts
        .counts
        .iter()
        .skip(1)
        .map(|&count| (count as f64 + ALPHA) / total)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_any_separator() {
        assert_eq!(parse_ints("12, 7\n300;4 x9"), vec![12, 7, 300, 4, 9]);
        assert_eq!(parse_ints(""), Vec::<i64>::new());
        assert_eq!(parse_ints("99999999999999999999999 1").len(), 2);
    }

    #[test]
    fn validation_drops_strays_truncates_and_rejects_short_answers() {
        let mut values: Vec<i64> = (1..=100).collect();
        values.push(0);
        let sample = validate(&values, 80, 355).unwrap();
        assert_eq!(sample.len(), 80);
        assert_eq!(sample[0], 1);
        let mut bad: Vec<i64> = (1..=100).collect();
        bad.extend([400; 10]);
        assert_eq!(validate(&bad, 100, 355), Err("out_of_range"));
        assert_eq!(
            validate(&(1..=50).collect::<Vec<_>>(), 60, 355),
            Err("too_few")
        );
        assert_eq!(
            validate(&(1..=90).collect::<Vec<_>>(), 200, 355),
            Err("too_few")
        );
        assert_eq!(validate(&[], 100, 355), Err("no_numbers"));
    }

    #[test]
    fn likelihood_prefers_the_matching_distribution() {
        let a: Vec<u16> = (0..300).map(|i| 1 + (i % 10) as u16).collect();
        let b: Vec<u16> = (0..300).map(|i| 100 + (i % 10) as u16).collect();
        let ca = Counts::from_samples(355, [a.as_slice()]);
        let cb = Counts::from_samples(355, [b.as_slice()]);
        assert!(mean_loglik(&a, &ca.log_probs()) > mean_loglik(&a, &cb.log_probs()));
        assert!(jsd(&ca, &cb) > jsd(&ca, &ca));
        assert!(jsd(&ca, &ca).abs() < 1e-12);
    }
}
