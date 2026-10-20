//! 可编辑的测评定义、判分配置与内容版本，不依赖调度或上游调用

use gateway_core::{account::ProviderAccountId, policy::ClientApiKeyId, routing::PublicModelId};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::EvaluationKind;
use crate::model::AdminError;

pub const DEFINITION_GRADER_VERSION: &str = "evaluation-definition-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationOutputFormat {
    Text,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationAnswerSource {
    FullText,
    FinalInteger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EvaluationGrader {
    None,
    Exact {
        source: EvaluationAnswerSource,
        expected: String,
    },
    Regex {
        pattern: String,
    },
    Model {
        config: EvaluationJudgeConfig,
    },
    Typesafe {
        config: super::TypeSafeJudgeConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationJudgeConfig {
    pub client_key_id: String,
    pub account_id: String,
    pub model: PublicModelId,
    pub reasoning_effort: Option<String>,
    pub timeout_seconds: u16,
    pub rubric: String,
    pub reference_answer: Option<String>,
}

impl EvaluationJudgeConfig {
    pub fn validate(&self, tested_accounts: &[String]) -> Result<(), AdminError> {
        validate_judge_connection(
            &self.client_key_id,
            &self.account_id,
            self.reasoning_effort.as_deref(),
            self.timeout_seconds,
        )?;
        if tested_accounts.contains(&self.account_id) {
            return Err(AdminError::invalid("判题账号不能同时作为本次被测账号"));
        }
        validate_text(&self.rubric, 16_384, "请填写评分标准，最多 16384 字节")?;
        if self
            .reference_answer
            .as_ref()
            .is_some_and(|answer| answer.len() > 32_768)
        {
            return Err(AdminError::invalid("参考答案最多 32768 字节"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationDefinition {
    pub name: String,
    pub prompt: String,
    pub output_format: EvaluationOutputFormat,
    pub grader: EvaluationGrader,
}

impl EvaluationDefinition {
    #[must_use]
    pub fn preset(kind: EvaluationKind) -> Self {
        Self {
            name: kind.label().to_owned(),
            prompt: kind.prompt().to_owned(),
            output_format: match kind {
                EvaluationKind::Candy => EvaluationOutputFormat::Text,
                EvaluationKind::Pelican => EvaluationOutputFormat::Html,
            },
            grader: match kind {
                EvaluationKind::Candy => EvaluationGrader::Exact {
                    source: EvaluationAnswerSource::FinalInteger,
                    expected: "21".to_owned(),
                },
                EvaluationKind::Pelican => EvaluationGrader::None,
            },
        }
    }

    pub fn validate(&self, tested_accounts: &[String]) -> Result<(), AdminError> {
        validate_text(&self.name, 128, "请填写测评名称，最多 128 字节")?;
        if self.name.chars().any(char::is_control) {
            return Err(AdminError::invalid("测评名称不能包含控制字符"));
        }
        validate_text(&self.prompt, 32_768, "请填写提示词，最多 32768 字节")?;
        match &self.grader {
            EvaluationGrader::None => {}
            EvaluationGrader::Exact { source, expected } => {
                validate_text(expected, 4096, "请填写预期答案，最多 4096 字节")?;
                if *source == EvaluationAnswerSource::FinalInteger
                    && (expected.parse::<u32>().is_err()
                        || !expected.bytes().all(|byte| byte.is_ascii_digit()))
                {
                    return Err(AdminError::invalid("最终整数答案必须是有效的非负整数"));
                }
            }
            EvaluationGrader::Regex { pattern } => {
                validate_text(pattern, 4096, "请填写正则表达式，最多 4096 字节")?;
                compile_pattern(pattern)?;
            }
            EvaluationGrader::Model { config } => config.validate(tested_accounts)?,
            EvaluationGrader::Typesafe { config } => config.validate()?,
        }
        Ok(())
    }

    /// 名称不改变实验条件，题目、规则与判题连接的变化必须分开比较
    #[must_use]
    pub fn version(&self) -> String {
        let content = serde_json::json!({
            "prompt": self.prompt,
            "outputFormat": self.output_format,
            "grader": self.grader,
            "graderVersion": DEFINITION_GRADER_VERSION,
        });
        hex::encode(Sha256::digest(content.to_string().as_bytes()))
    }
}

pub(super) fn compile_pattern(pattern: &str) -> Result<regex::Regex, AdminError> {
    regex::RegexBuilder::new(pattern)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .build()
        .map_err(|_| AdminError::invalid("正则表达式无效或过于复杂"))
}

pub(super) fn validate_effort(effort: Option<&str>) -> Result<(), AdminError> {
    if effort.is_some_and(|value| {
        value.is_empty() || value.len() > 32 || !value.bytes().all(|c| c.is_ascii_lowercase())
    }) {
        return Err(AdminError::invalid("推理强度格式无效"));
    }
    Ok(())
}

fn validate_text(value: &str, max: usize, message: &str) -> Result<(), AdminError> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        return Err(AdminError::invalid(message));
    }
    Ok(())
}

pub(super) fn validate_judge_connection(
    client_key_id: &str,
    account_id: &str,
    effort: Option<&str>,
    timeout_seconds: u16,
) -> Result<(), AdminError> {
    ClientApiKeyId::new(client_key_id.to_owned())
        .map_err(|_| AdminError::invalid("判题 Key ID 无效"))?;
    ProviderAccountId::new(account_id.to_owned())
        .map_err(|_| AdminError::invalid("判题账号 ID 无效"))?;
    validate_effort(effort)?;
    if !(1..=600).contains(&timeout_seconds) {
        return Err(AdminError::invalid("判题超时必须在 1 到 600 秒之间"));
    }
    Ok(())
}
