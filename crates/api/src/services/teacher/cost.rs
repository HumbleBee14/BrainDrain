//! Cost estimate for a hosted-teacher logprob extraction run.
//!
//! Shown before the user commits to a higher-fidelity run, so the estimate is
//! deliberately pessimistic in every direction it can be: conservative
//! throughput, a startup allowance for pulling and loading tens of gigabytes of
//! weights, and — when the dataset has not been measured yet — a generous
//! tokens-per-pair guess. An estimate that lands under the real bill is a
//! broken promise; one that lands over it is a pleasant surprise.

use serde::Serialize;
use ts_rs::TS;
use utoipa::ToSchema;

/// GPU time allowed for pulling weights into the cache and loading the model
/// before any token is scored. Cache-warm runs beat this comfortably.
const STARTUP_ALLOWANCE_HOURS: f64 = 0.1;

/// Scored tokens assumed per pair when the dataset has no measured counts.
/// Only completion positions are scored, and generated answers in this platform
/// run well short of this.
const APPROX_SCORED_TOKENS_PER_PAIR: i64 = 200;

/// Whether the token counts behind an estimate were measured or guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS, ToSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EstimateBasis {
    /// Counted with the run's actual tokenizer.
    Measured,
    /// Derived from the dataset's pair count; the run bills what it uses.
    Approximate,
}

/// What a fidelity upgrade will cost, as shown next to the opt-in.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ExtractionEstimate {
    pub scored_tokens: i64,
    pub est_gpu_hours: f64,
    pub est_cost_usd: f64,
    pub basis: EstimateBasis,
    pub gpu_class: String,
}

/// Scored-token count for a dataset, preferring measured counts and falling
/// back to the pair count.
///
/// Token counts are a property of a (dataset, tokenizer) pair, so a stored
/// count is only valid for the tokenizer recorded in
/// `datasets.token_count_tokenizer_hash`. Nothing writes those columns yet —
/// measuring them means rendering the dataset under a real tokenizer, which
/// only the workers can do — so in practice every estimate today reports the
/// approximate basis, and the UI labels it as approximate. The measured arm
/// goes live unchanged the moment a writer lands.
pub fn scored_tokens_for(measured: Option<i64>, pair_count: Option<i32>) -> (i64, EstimateBasis) {
    match measured {
        Some(tokens) if tokens > 0 => (tokens, EstimateBasis::Measured),
        _ => (
            pair_count.unwrap_or(0).max(0) as i64 * APPROX_SCORED_TOKENS_PER_PAIR,
            EstimateBasis::Approximate,
        ),
    }
}

/// Estimate the GPU time and dollar cost of scoring `scored_tokens`.
pub fn estimate_extraction(
    scored_tokens: i64,
    basis: EstimateBasis,
    tokens_per_sec: f64,
    gpu_hourly_rate: f64,
    gpu_class: &str,
) -> ExtractionEstimate {
    let scoring_hours = if tokens_per_sec > 0.0 {
        scored_tokens.max(0) as f64 / tokens_per_sec / 3600.0
    } else {
        0.0
    };
    let est_gpu_hours = round_to(scoring_hours + STARTUP_ALLOWANCE_HOURS, 4);
    ExtractionEstimate {
        scored_tokens,
        est_gpu_hours,
        est_cost_usd: round_to(est_gpu_hours * gpu_hourly_rate, 2),
        basis,
        gpu_class: gpu_class.to_string(),
    }
}

fn round_to(value: f64, places: u32) -> f64 {
    let factor = 10_f64.powi(places as i32);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_computed_case() {
        // 1_800_000 tokens at 1000 tok/s = 1800 s = 0.5 h, plus the 0.1 h
        // startup allowance, at $3.00/h.
        let estimate =
            estimate_extraction(1_800_000, EstimateBasis::Measured, 1000.0, 3.00, "a10080gb");
        assert_eq!(estimate.est_gpu_hours, 0.6);
        assert_eq!(estimate.est_cost_usd, 1.80);
        assert_eq!(estimate.basis, EstimateBasis::Measured);
    }

    #[test]
    fn empty_dataset_still_bills_the_startup_allowance() {
        let estimate = estimate_extraction(0, EstimateBasis::Approximate, 1500.0, 3.00, "a10080gb");
        assert_eq!(estimate.est_gpu_hours, STARTUP_ALLOWANCE_HOURS);
        assert!(estimate.est_cost_usd > 0.0);
    }

    #[test]
    fn zero_throughput_does_not_divide_by_zero() {
        let estimate = estimate_extraction(1000, EstimateBasis::Measured, 0.0, 3.00, "h100");
        assert!(estimate.est_gpu_hours.is_finite());
        assert_eq!(estimate.est_gpu_hours, STARTUP_ALLOWANCE_HOURS);
    }

    #[test]
    fn measured_count_is_used_when_present() {
        let (tokens, basis) = scored_tokens_for(Some(500_000), Some(100));
        assert_eq!(tokens, 500_000);
        assert_eq!(basis, EstimateBasis::Measured);
    }

    #[test]
    fn missing_measurement_falls_back_to_pair_count() {
        let (tokens, basis) = scored_tokens_for(None, Some(250));
        assert_eq!(tokens, 250 * APPROX_SCORED_TOKENS_PER_PAIR);
        assert_eq!(basis, EstimateBasis::Approximate);
    }

    #[test]
    fn absent_pair_count_and_measurement_yields_zero_tokens() {
        let (tokens, basis) = scored_tokens_for(None, None);
        assert_eq!(tokens, 0);
        assert_eq!(basis, EstimateBasis::Approximate);
    }

    #[test]
    fn negative_pair_count_is_clamped() {
        let (tokens, _) = scored_tokens_for(None, Some(-5));
        assert_eq!(tokens, 0);
    }

    #[test]
    fn nonpositive_measured_count_is_not_trusted() {
        let (tokens, basis) = scored_tokens_for(Some(0), Some(10));
        assert_eq!(tokens, 10 * APPROX_SCORED_TOKENS_PER_PAIR);
        assert_eq!(basis, EstimateBasis::Approximate);
    }
}
