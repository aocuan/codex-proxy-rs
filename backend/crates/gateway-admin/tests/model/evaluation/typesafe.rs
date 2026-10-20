//! TypeSafe 概率边界与判题配置的领域约束

use gateway_admin::model::evaluation::{EvaluationVerdict, TypeSafeJudgeConfig};

#[test]
fn typesafe_probability_preserves_uncertainty_and_rejects_invalid_values() {
    let config = TypeSafeJudgeConfig {
        model: "jev-1.13.0".to_owned(),
        decision_threshold: 80,
        timeout_seconds: 60,
        rubric: "回答符合题目要求".to_owned(),
        reference_answer: None,
    };
    for (probability, expected) in [
        (0.0, EvaluationVerdict::Wrong),
        (0.2, EvaluationVerdict::Wrong),
        (0.21, EvaluationVerdict::Ungraded),
        (0.79, EvaluationVerdict::Ungraded),
        (0.8, EvaluationVerdict::Correct),
        (1.0, EvaluationVerdict::Correct),
    ] {
        assert_eq!(config.verdict(probability).unwrap(), expected);
    }
    for probability in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        assert!(config.verdict(probability).is_err());
    }
    for threshold in 51..=100 {
        let boundary = TypeSafeJudgeConfig {
            decision_threshold: threshold,
            ..config.clone()
        };
        assert_eq!(
            boundary.verdict(f64::from(threshold) / 100.0).unwrap(),
            EvaluationVerdict::Correct
        );
        assert_eq!(
            boundary
                .verdict(f64::from(100 - threshold) / 100.0)
                .unwrap(),
            EvaluationVerdict::Wrong
        );
    }
    for threshold in [0, 50, 101] {
        assert!(
            TypeSafeJudgeConfig {
                decision_threshold: threshold,
                ..config.clone()
            }
            .validate()
            .is_err()
        );
    }
}
