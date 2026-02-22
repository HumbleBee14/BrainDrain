//! Centralized token and cost estimation utilities.
//!
//! All estimation logic lives here so that upgrading to model-specific
//! tokenizers or per-model pricing only requires changing this file.

/// Approximate characters per token for fallback billing estimation.
/// Used when actual token counts are unavailable (e.g. streaming client disconnect).
const CHARS_PER_TOKEN: i64 = 4;

/// Estimate token count from a list of chat messages.
///
/// Sums character lengths across all messages and converts to an
/// approximate token count. Does not account for chat template overhead.
pub fn estimate_tokens_from_messages<'a>(messages: impl Iterator<Item = &'a str>) -> i64 {
    let total_chars: i64 = messages.map(|m| m.len() as i64).sum();
    total_chars / CHARS_PER_TOKEN
}

// ── Cost estimation ──

/// Per-million-token pricing for input (prompt) tokens.
const INPUT_COST_PER_MILLION: f64 = 0.15;

/// Per-million-token pricing for output (completion) tokens.
const OUTPUT_COST_PER_MILLION: f64 = 0.60;

/// Estimate inference cost in USD from token counts.
///
/// Uses flat-rate pricing. To support per-model pricing in the future,
/// add a `model: &str` parameter and a lookup table here.
pub fn estimate_inference_cost(tokens_in: i64, tokens_out: i64) -> f64 {
    let input_cost = tokens_in as f64 * INPUT_COST_PER_MILLION / 1_000_000.0;
    let output_cost = tokens_out as f64 * OUTPUT_COST_PER_MILLION / 1_000_000.0;
    input_cost + output_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_from_messages() {
        let messages = vec!["hello", "world"]; // 5 + 5 = 10 chars / 4 = 2
        assert_eq!(estimate_tokens_from_messages(messages.into_iter()), 2);

        let empty: Vec<&str> = vec![];
        assert_eq!(estimate_tokens_from_messages(empty.into_iter()), 0);
    }

    #[test]
    fn test_estimate_inference_cost() {
        // 1M input + 1M output = $0.15 + $0.60 = $0.75
        let cost = estimate_inference_cost(1_000_000, 1_000_000);
        assert!((cost - 0.75).abs() < f64::EPSILON);

        // 0 tokens = $0
        assert_eq!(estimate_inference_cost(0, 0), 0.0);
    }

    #[test]
    fn test_cost_scales_linearly() {
        let cost_1k = estimate_inference_cost(1_000, 1_000);
        let cost_2k = estimate_inference_cost(2_000, 2_000);
        assert!((cost_2k - cost_1k * 2.0).abs() < f64::EPSILON);
    }
}
