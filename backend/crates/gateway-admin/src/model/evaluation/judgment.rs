//! 独立 AI 判题任务及结构化结论，判题故障不改变被测请求状态

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{EvaluationDefinition, EvaluationItemStatus, EvaluationObservation, EvaluationVerdict};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationJudgment {
    pub id: String,
    pub item_id: String,
    pub account_id: Option<String>,
    pub attempt: i32,
    pub status: EvaluationItemStatus,
    pub request_id: Option<String>,
    pub verdict: EvaluationVerdict,
    pub reason: Option<String>,
    pub observation: EvaluationObservation,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ClaimedEvaluationJudgment {
    pub judgment: EvaluationJudgment,
    pub run_id: String,
    pub definition: EvaluationDefinition,
    pub response_text: String,
}

#[derive(Debug, Clone)]
pub struct EvaluationJudgmentCompletion {
    pub status: EvaluationItemStatus,
    pub verdict: EvaluationVerdict,
    pub reason: String,
    pub observation: EvaluationObservation,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgeReply {
    #[serde(deserialize_with = "Option::<bool>::deserialize")]
    pass: Option<bool>,
    reason: String,
}

/// 只接受完整的单个 JSON 结论，模型拒绝或证据不足可返回 null
pub fn parse_judgment(response: &str) -> Result<(EvaluationVerdict, String), &'static str> {
    if response.len() > 65_536 {
        return Err("判题回答超过大小限制");
    }
    let source = response.trim();
    let source = source
        .strip_prefix("```json")
        .and_then(|body| body.strip_suffix("```"))
        .map_or(source, str::trim);
    let reply: JudgeReply =
        serde_json::from_str(source).map_err(|_| "判题模型未返回有效的 JSON 结论")?;
    if reply.reason.trim().is_empty() || reply.reason.len() > 4096 {
        return Err("判题理由为空或过长");
    }
    Ok((
        match reply.pass {
            Some(true) => EvaluationVerdict::Correct,
            Some(false) => EvaluationVerdict::Wrong,
            None => EvaluationVerdict::Ungraded,
        },
        reply.reason,
    ))
}
