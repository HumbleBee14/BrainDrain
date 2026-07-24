//! Deployment eval-gate policy.
//!
//! A pure, config-driven check that decides whether a model's evaluation
//! scores clear the thresholds required to deploy it to production. The policy
//! is a list of rules; each rule names a metric extracted from an evaluation's
//! `scores` JSON and a threshold that metric must satisfy. Rules are data, so a
//! new score type can be gated later by adding a [`GateMetric`] variant plus a
//! config field — no database schema change (scores is JSONB) and no change to
//! this module's control flow.

use serde_json::Value;

/// Which side of the threshold a metric must fall on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDirection {
    /// The metric must be greater than or equal to the threshold.
    AtLeast,
    /// The metric must be less than or equal to the threshold.
    AtMost,
}

/// A metric that can be extracted from an evaluation's `scores` JSON.
///
/// Each variant knows how to pull its own value out of the scores object,
/// which is what keeps the policy a score-key → threshold map rather than a
/// bag of hardcoded fields. Adding a variant is the only change needed to gate
/// on a new score type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMetric {
    /// A/B win rate against the base model, `0.0`–`1.0`
    /// (`scores.ab_comparison.win_rate`).
    AbWinRate,
    /// General-benchmark regression against the base model, in percentage
    /// points: `base_score - finetuned_score`. Positive means the fine-tuned
    /// model scored *worse* than the base on general capability. Derived from
    /// `scores.general.delta_pct` (which the evaluator stores as
    /// `finetuned - base`).
    BenchmarkRegression,
    /// Document-knowledge lift over the base model on the golden holdout set,
    /// as a judged-mean difference on a 1–5 rubric (roughly -4.0–4.0)
    /// (`scores.doc_knowledge.knowledge_lift`).
    DocKnowledgeLift,
}

impl GateMetric {
    /// Stable, human-readable label used in violation messages.
    pub fn label(self) -> &'static str {
        match self {
            GateMetric::AbWinRate => "A/B win rate",
            GateMetric::BenchmarkRegression => "general-benchmark regression",
            GateMetric::DocKnowledgeLift => "document-knowledge lift",
        }
    }

    /// Extract this metric's value from an evaluation `scores` object, or
    /// `None` when the evaluation did not produce it (suite skipped, no eval
    /// yet, or the metric is null).
    pub fn extract(self, scores: &Value) -> Option<f64> {
        match self {
            GateMetric::AbWinRate => scores
                .get("ab_comparison")
                .and_then(|v| v.get("win_rate"))
                .and_then(Value::as_f64),
            GateMetric::BenchmarkRegression => scores
                .get("general")
                .and_then(|v| v.get("delta_pct"))
                .and_then(Value::as_f64)
                .map(|delta| -delta),
            GateMetric::DocKnowledgeLift => scores
                .get("doc_knowledge")
                .and_then(|v| v.get("knowledge_lift"))
                .and_then(Value::as_f64),
        }
    }
}

/// One gate rule: a metric, the direction it must satisfy, and the threshold.
#[derive(Debug, Clone, Copy)]
pub struct GateRule {
    pub metric: GateMetric,
    pub direction: GateDirection,
    pub threshold: f64,
}

impl GateRule {
    /// Check a metric value against this rule. `Ok(())` when it satisfies the
    /// rule, otherwise a human-readable reason for the failure.
    fn check_value(&self, value: f64) -> Result<(), String> {
        let ok = match self.direction {
            GateDirection::AtLeast => value >= self.threshold,
            GateDirection::AtMost => value <= self.threshold,
        };
        if ok {
            return Ok(());
        }
        let cmp = match self.direction {
            GateDirection::AtLeast => "below the minimum",
            GateDirection::AtMost => "above the maximum",
        };
        Err(format!(
            "{} is {value:.3}, {cmp} of {:.3}",
            self.metric.label(),
            self.threshold
        ))
    }
}

/// Why a specific rule blocked the deploy.
#[derive(Debug, Clone)]
pub struct GateViolation {
    pub metric: GateMetric,
    pub reason: String,
}

/// Outcome of checking a model's scores against the policy.
#[derive(Debug, Clone)]
pub enum GateDecision {
    /// No rules configured — the gate is disabled and deploy proceeds.
    Disabled,
    /// Every configured rule was satisfied.
    Passed,
    /// One or more rules failed (or a required metric was unavailable).
    Blocked(Vec<GateViolation>),
}

/// The deployment eval-gate policy: a set of rules assembled from config.
#[derive(Debug, Clone, Default)]
pub struct DeployGatePolicy {
    rules: Vec<GateRule>,
}

impl DeployGatePolicy {
    /// Build a policy from the configured thresholds. Each `None` threshold
    /// contributes no rule, so an all-`None` config yields an empty (disabled)
    /// policy that never blocks a deploy — preserving the pre-gate behavior.
    pub fn from_thresholds(
        min_ab_win_rate: Option<f64>,
        max_benchmark_regression: Option<f64>,
        min_doc_knowledge_lift: Option<f64>,
    ) -> Self {
        let mut rules = Vec::new();
        if let Some(threshold) = min_ab_win_rate {
            rules.push(GateRule {
                metric: GateMetric::AbWinRate,
                direction: GateDirection::AtLeast,
                threshold,
            });
        }
        if let Some(threshold) = max_benchmark_regression {
            rules.push(GateRule {
                metric: GateMetric::BenchmarkRegression,
                direction: GateDirection::AtMost,
                threshold,
            });
        }
        if let Some(threshold) = min_doc_knowledge_lift {
            rules.push(GateRule {
                metric: GateMetric::DocKnowledgeLift,
                direction: GateDirection::AtLeast,
                threshold,
            });
        }
        Self { rules }
    }

    /// Whether any rule is configured.
    pub fn is_enabled(&self) -> bool {
        !self.rules.is_empty()
    }

    /// Check a model's evaluation `scores` against the policy.
    ///
    /// A configured rule whose metric is absent from `scores` is treated as a
    /// violation, not a pass: the gate exists to require positive evidence of
    /// quality, so an unproven model (no eval, or an eval that did not produce
    /// the metric) is blocked rather than waved through.
    pub fn check(&self, scores: &Value) -> GateDecision {
        if self.rules.is_empty() {
            return GateDecision::Disabled;
        }
        let mut violations = Vec::new();
        for rule in &self.rules {
            match rule.metric.extract(scores) {
                Some(value) => {
                    if let Err(reason) = rule.check_value(value) {
                        violations.push(GateViolation {
                            metric: rule.metric,
                            reason,
                        });
                    }
                }
                None => violations.push(GateViolation {
                    metric: rule.metric,
                    reason: format!(
                        "{} is not available — run an evaluation for this model before deploying",
                        rule.metric.label()
                    ),
                }),
            }
        }
        if violations.is_empty() {
            GateDecision::Passed
        } else {
            GateDecision::Blocked(violations)
        }
    }
}

/// Render the blocking violations into a single operator-facing message.
pub fn format_block_message(violations: &[GateViolation]) -> String {
    let reasons: Vec<&str> = violations.iter().map(|v| v.reason.as_str()).collect();
    format!(
        "Deployment blocked by the evaluation gate: {}. Deploy is allowed once the model meets \
         the configured thresholds (re-evaluate after improving it), or an operator disables the \
         gate. Rollbacks to a previously deployed version are not gated.",
        reasons.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scores(win_rate: f64, delta_pct: f64) -> Value {
        json!({
            "ab_comparison": {"win_rate": win_rate},
            "general": {"delta_pct": delta_pct},
            "doc_knowledge": {"knowledge_lift": 1.5},
            "overall": 80.0,
        })
    }

    #[test]
    fn empty_policy_is_disabled_and_never_blocks() {
        let policy = DeployGatePolicy::from_thresholds(None, None, None);
        assert!(!policy.is_enabled());
        assert!(matches!(policy.check(&json!({})), GateDecision::Disabled));
        // Even against terrible scores, a disabled gate does not block.
        assert!(matches!(
            policy.check(&scores(0.0, -50.0)),
            GateDecision::Disabled
        ));
    }

    #[test]
    fn win_rate_at_or_above_threshold_passes() {
        let policy = DeployGatePolicy::from_thresholds(Some(0.5), None, None);
        assert!(matches!(
            policy.check(&scores(0.5, 0.0)),
            GateDecision::Passed
        ));
        assert!(matches!(
            policy.check(&scores(0.9, 0.0)),
            GateDecision::Passed
        ));
    }

    #[test]
    fn win_rate_below_threshold_blocks() {
        let policy = DeployGatePolicy::from_thresholds(Some(0.6), None, None);
        match policy.check(&scores(0.4, 0.0)) {
            GateDecision::Blocked(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].metric, GateMetric::AbWinRate);
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn regression_within_budget_passes() {
        // delta_pct = -5 => regression = 5, budget = 10 => allowed.
        let policy = DeployGatePolicy::from_thresholds(None, Some(10.0), None);
        assert!(matches!(
            policy.check(&scores(1.0, -5.0)),
            GateDecision::Passed
        ));
        // Improvement (positive delta) is a negative regression, always allowed.
        assert!(matches!(
            policy.check(&scores(1.0, 12.0)),
            GateDecision::Passed
        ));
    }

    #[test]
    fn regression_over_budget_blocks() {
        // delta_pct = -15 => regression = 15, budget = 10 => blocked.
        let policy = DeployGatePolicy::from_thresholds(None, Some(10.0), None);
        match policy.check(&scores(1.0, -15.0)) {
            GateDecision::Blocked(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].metric, GateMetric::BenchmarkRegression);
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn both_rules_can_fail_together() {
        let policy = DeployGatePolicy::from_thresholds(Some(0.6), Some(10.0), None);
        match policy.check(&scores(0.3, -20.0)) {
            GateDecision::Blocked(v) => assert_eq!(v.len(), 2),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn doc_knowledge_lift_at_or_above_threshold_passes() {
        // scores() helper carries knowledge_lift = 1.5.
        let policy = DeployGatePolicy::from_thresholds(None, None, Some(1.0));
        assert!(matches!(
            policy.check(&scores(0.0, -50.0)),
            GateDecision::Passed
        ));
    }

    #[test]
    fn doc_knowledge_lift_below_threshold_blocks() {
        let policy = DeployGatePolicy::from_thresholds(None, None, Some(2.0));
        match policy.check(&scores(1.0, 0.0)) {
            GateDecision::Blocked(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].metric, GateMetric::DocKnowledgeLift);
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn null_knowledge_lift_is_treated_as_missing() {
        // Suite skipped (no golden set) stores knowledge_lift: null.
        let policy = DeployGatePolicy::from_thresholds(None, None, Some(0.5));
        let s = json!({"doc_knowledge": {"knowledge_lift": null}});
        match policy.check(&s) {
            GateDecision::Blocked(v) => {
                assert!(v[0].reason.contains("not available"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn missing_metric_blocks_rather_than_waves_through() {
        // Gate enabled but scores lack the required metric (e.g. no eval run).
        let policy = DeployGatePolicy::from_thresholds(Some(0.6), None, None);
        match policy.check(&json!({})) {
            GateDecision::Blocked(v) => {
                assert_eq!(v.len(), 1);
                assert!(v[0].reason.contains("not available"));
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn null_metric_is_treated_as_missing() {
        // A/B suite ran but produced no win rate (no validation data).
        let policy = DeployGatePolicy::from_thresholds(Some(0.6), None, None);
        let s = json!({"ab_comparison": {"win_rate": null}});
        assert!(matches!(policy.check(&s), GateDecision::Blocked(_)));
    }

    #[test]
    fn block_message_lists_every_reason() {
        let policy = DeployGatePolicy::from_thresholds(Some(0.6), Some(10.0), None);
        if let GateDecision::Blocked(v) = policy.check(&scores(0.3, -20.0)) {
            let msg = format_block_message(&v);
            assert!(msg.contains("A/B win rate"));
            assert!(msg.contains("general-benchmark regression"));
            assert!(msg.contains("Rollbacks"));
        } else {
            panic!("expected Blocked");
        }
    }
}
