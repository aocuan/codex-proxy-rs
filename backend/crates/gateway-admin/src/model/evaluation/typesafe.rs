//! TypeSafe 判题参数及概率阈值策略，不生成模型未提供的解释

use serde::{Deserialize, Serialize};

use super::EvaluationVerdict;
use crate::model::AdminError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeSafeJudgeConfig {
    pub model: String,
    pub decision_threshold: u8,
    pub timeout_seconds: u16,
    pub rubric: String,
    pub reference_answer: Option<String>,
}

impl TypeSafeJudgeConfig {
    pub fn validate(&self) -> Result<(), AdminError> {
        validate_connection(&self.model, self.decision_threshold, self.timeout_seconds)?;
        if self.rubric.trim().is_empty()
            || self.rubric.len() > 16_384
            || self
                .reference_answer
                .as_ref()
                .is_some_and(|text| text.len() > 32_768)
        {
            return Err(AdminError::invalid(
                "TypeSafe 模型、判定阈值、超时或评分标准无效",
            ));
        }
        Ok(())
    }

    pub fn verdict(&self, probability: f64) -> Result<EvaluationVerdict, &'static str> {
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err("TypeSafe 返回了无效概率");
        }
        let threshold = f64::from(self.decision_threshold) / 100.0;
        let fail_threshold = f64::from(100 - self.decision_threshold) / 100.0;
        Ok(if probability >= threshold {
            EvaluationVerdict::Correct
        } else if probability <= fail_threshold {
            EvaluationVerdict::Wrong
        } else {
            EvaluationVerdict::Ungraded
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct TypeSafeJudgmentResponse {
    pub model: String,
    pub answers: TypeSafeAnswers,
    pub usage: TypeSafeUsage,
}

#[derive(Debug, Deserialize)]
pub struct TypeSafeAnswers {
    pub meets_criteria: TypeSafeAnswer,
}

#[derive(Debug, Deserialize)]
pub struct TypeSafeAnswer {
    #[serde(rename = "type")]
    pub question_type: String,
    pub noul: f64,
}

#[derive(Debug, Deserialize)]
pub struct TypeSafeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationJudgeTarget {
    Account(String),
    TypeSafe,
}

pub(super) fn validate_connection(
    model: &str,
    decision_threshold: u8,
    timeout_seconds: u16,
) -> Result<(), AdminError> {
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
        || !(51..=100).contains(&decision_threshold)
        || !(1..=120).contains(&timeout_seconds)
    {
        return Err(AdminError::invalid("TypeSafe 模型、判定阈值或超时无效"));
    }
    Ok(())
}
