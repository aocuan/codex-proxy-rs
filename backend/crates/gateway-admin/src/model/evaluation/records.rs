//! 测评任务的不可变实验参数、持久状态与计划游标

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use gateway_core::{
    account::ProviderAccountId, policy::ClientApiKeyId, routing::PublicModelId,
    time::DeploymentTimeZone,
};
use serde::{Deserialize, Serialize};

use super::{
    EvaluationDefinition, EvaluationKind, EvaluationVerdict, GRADER_VERSION, MAX_ACCOUNTS_PER_RUN,
    MAX_PLAN_INTERVAL_MINUTES, MAX_REPETITIONS, REQUEST_PROFILE_VERSION,
};
use crate::model::AdminError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationParameters {
    pub kind: EvaluationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<EvaluationDefinition>,
    pub client_key_id: String,
    pub account_ids: Vec<String>,
    pub model: PublicModelId,
    pub reasoning_effort: Option<String>,
    pub repetitions: u8,
    pub timeout_seconds: u16,
}

impl EvaluationParameters {
    /// 提交与执行前都校验范围；权限、预算及账号可用性仍以 Core 当次准入为准
    pub fn validate(&self) -> Result<(), AdminError> {
        ClientApiKeyId::new(self.client_key_id.clone())
            .map_err(|_| AdminError::invalid("Client Key ID 无效"))?;
        if self.account_ids.is_empty() || self.account_ids.len() > MAX_ACCOUNTS_PER_RUN {
            return Err(AdminError::invalid("每次选择 1 到 20 个账号"));
        }
        let mut unique = BTreeSet::new();
        for account_id in &self.account_ids {
            ProviderAccountId::new(account_id.clone())
                .map_err(|_| AdminError::invalid("账号 ID 无效"))?;
            if !unique.insert(account_id) {
                return Err(AdminError::invalid("账号不能重复"));
            }
        }
        if !(1..=MAX_REPETITIONS).contains(&self.repetitions)
            || (self.definition.is_none()
                && self.kind == EvaluationKind::Pelican
                && self.repetitions != 1)
        {
            return Err(AdminError::invalid(
                "糖果题可重复 1 到 5 次，画鹈鹕每次执行 1 次",
            ));
        }
        if !(1..=600).contains(&self.timeout_seconds) {
            return Err(AdminError::invalid("单次请求超时必须在 1 到 600 秒之间"));
        }
        super::definition::validate_effort(self.reasoning_effort.as_deref())?;
        if let Some(definition) = &self.definition {
            definition.validate(&self.account_ids)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn item_count(&self) -> usize {
        self.account_ids.len() * usize::from(self.repetitions)
    }
}

/// 历史记录冻结题目和版本，后续升级不能改变旧实验的含义
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<EvaluationDefinition>,
    pub prompt: String,
    pub prompt_version: String,
    pub request_profile_version: String,
    pub grader_version: String,
    pub gateway_version: String,
}

impl EvaluationProfile {
    #[must_use]
    pub fn current(kind: EvaluationKind, gateway_version: &str) -> Self {
        Self {
            definition: None,
            prompt: kind.prompt().to_owned(),
            prompt_version: kind.prompt_version().to_owned(),
            request_profile_version: REQUEST_PROFILE_VERSION.to_owned(),
            grader_version: GRADER_VERSION.to_owned(),
            gateway_version: gateway_version.to_owned(),
        }
    }

    #[must_use]
    pub fn for_parameters(parameters: &EvaluationParameters, gateway_version: &str) -> Self {
        let Some(definition) = &parameters.definition else {
            return Self::current(parameters.kind, gateway_version);
        };
        Self {
            definition: Some(definition.clone()),
            prompt: definition.prompt.clone(),
            prompt_version: definition.version(),
            request_profile_version: REQUEST_PROFILE_VERSION.to_owned(),
            grader_version: super::DEFINITION_GRADER_VERSION.to_owned(),
            gateway_version: gateway_version.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationRunSeed {
    pub id: String,
    pub parameters: EvaluationParameters,
    pub profile: EvaluationProfile,
    pub created_at: DateTime<Utc>,
}

impl EvaluationRunSeed {
    #[must_use]
    pub fn new(
        parameters: EvaluationParameters,
        gateway_version: &str,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: format!("eval_{}", uuid::Uuid::now_v7().simple()),
            profile: EvaluationProfile::for_parameters(&parameters, gateway_version),
            parameters,
            created_at: now,
        }
    }
}

/// 计数来自子任务查询，不另外维护可与子任务分歧的运行状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationRun {
    pub id: String,
    pub parameters: EvaluationParameters,
    pub profile: EvaluationProfile,
    pub plan_id: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub stop_requested: bool,
    pub created_at: DateTime<Utc>,
    pub total_items: i64,
    pub retained_items: i64,
    pub active_items: i64,
    pub succeeded_items: i64,
    pub correct_items: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationItemStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    Skipped,
}

impl EvaluationItemStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Skipped => "skipped",
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationItem {
    pub judgment: Option<super::EvaluationJudgment>,
    pub id: String,
    pub run_id: String,
    pub account_id: String,
    pub repetition: i32,
    pub status: EvaluationItemStatus,
    pub request_id: Option<String>,
    pub verdict: EvaluationVerdict,
    pub final_answer: Option<i64>,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub has_output: bool,
    pub has_artifact: bool,
    pub observation: EvaluationObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationRunDetail {
    pub run: EvaluationRun,
    pub items: Vec<EvaluationItem>,
}

/// 账号列表的轻量历史，不携带原始回答、作品或其他账号的实验参数
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationHistoryItem {
    pub name: String,
    pub profile_version: String,
    pub grader_type: String,
    pub judgment_status: Option<EvaluationItemStatus>,
    pub id: String,
    pub run_id: String,
    pub account_id: String,
    pub kind: EvaluationKind,
    pub model: PublicModelId,
    pub reasoning_effort: Option<String>,
    pub status: EvaluationItemStatus,
    pub verdict: EvaluationVerdict,
    pub final_answer: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedEvaluation {
    pub item: EvaluationItem,
    pub parameters: EvaluationParameters,
    pub profile: EvaluationProfile,
}

/// 原始输出先持久化，再允许提交 Core 的下游消费事实
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationOutput {
    pub response_text: String,
    pub artifact_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationCompletion {
    pub judge_target: Option<super::EvaluationJudgeTarget>,
    pub status: EvaluationItemStatus,
    pub verdict: EvaluationVerdict,
    pub final_answer: Option<i64>,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub ttft_ms: Option<i64>,
    pub duration_ms: i64,
    pub completed_at: DateTime<Utc>,
    pub observation: EvaluationObservation,
}

/// 已终结请求的只读观测快照；缺失值不估算，费用仍以 Core 用量记录为准
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EvaluationObservation {
    pub request_history_available: bool,
    pub provider: Option<String>,
    pub account_id: Option<String>,
    pub upstream_model: Option<String>,
    pub response_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_preset: Option<String>,
    pub service_tier: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cost_amount: Option<String>,
    pub cost_currency: Option<String>,
    pub cost_source: Option<String>,
}

impl From<crate::model::observability::UsageRecord> for EvaluationObservation {
    fn from(record: crate::model::observability::UsageRecord) -> Self {
        Self {
            request_history_available: true,
            provider: record.provider_kind,
            account_id: record.provider_account_ref,
            upstream_model: record.upstream_model_id,
            // 网关不再采集响应自报模型，保留字段用于历史记录与独立判题结果
            response_model: None,
            reasoning_effort: record.reasoning_effort,
            reasoning_preset: record.reasoning_preset,
            service_tier: record.service_tier,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cached_input_tokens: record.cached_input_tokens,
            reasoning_output_tokens: record.reasoning_output_tokens,
            cost_amount: record.cost_amount.map(|value| value.to_string()),
            cost_currency: record.cost_currency,
            cost_source: Some(record.cost_source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationPlanSettings {
    pub name: String,
    pub parameters: EvaluationParameters,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cron_expression: String,
    pub timezone: DeploymentTimeZone,
}

impl EvaluationPlanSettings {
    /// 每次保存重新计算未来游标，修改或恢复计划不补跑停用期间的请求
    pub fn next_run_after(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, AdminError> {
        self.parameters.validate()?;
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(AdminError::invalid("计划名称不能为空、过长或包含控制字符"));
        }
        let next = self.next_occurrence_after(now, now)?;
        Ok(self.enabled.then_some(next))
    }

    /// 从已保存的时段推进，跳过错过的间隔，不因巡检延迟累积漂移
    pub fn next_occurrence_after(
        &self,
        anchor: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, AdminError> {
        if let Some(minutes) = self.interval_minutes {
            if !(1..=MAX_PLAN_INTERVAL_MINUTES).contains(&minutes) {
                return Err(AdminError::invalid(format!(
                    "执行间隔必须在 1 到 {MAX_PLAN_INTERVAL_MINUTES} 分钟之间"
                )));
            }
            if !self.cron_expression.is_empty() {
                return Err(AdminError::invalid("分钟间隔不能与 Cron 同时设置"));
            }
            if anchor > now {
                return Ok(anchor);
            }
            let seconds = i64::from(minutes) * 60;
            let elapsed = now.signed_duration_since(anchor).num_seconds();
            let steps = elapsed / seconds + 1;
            return anchor
                .checked_add_signed(chrono::Duration::seconds(steps * seconds))
                .ok_or_else(|| AdminError::invalid("计划没有未来执行时间"));
        }
        // 已保存的日历计划继续按原时区运行，编辑为分钟间隔时不再保留表达式
        if self.cron_expression.len() > 128 || self.cron_expression.split_whitespace().count() != 5
        {
            return Err(AdminError::invalid("Cron 表达式必须是 5 段格式"));
        }
        let schedule =
            crate::model::schedule::CronSchedule::parse(&self.cron_expression, self.timezone)
                .map_err(|_| AdminError::invalid("Cron 表达式无效"))?;
        schedule
            .next_after(now)
            .ok_or_else(|| AdminError::invalid("计划没有未来执行时间"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationPlan {
    pub id: String,
    pub revision: i64,
    pub settings: EvaluationPlanSettings,
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SaveEvaluationPlan {
    pub id: String,
    pub expected_revision: Option<i64>,
    pub settings: EvaluationPlanSettings,
    pub next_run_at: Option<DateTime<Utc>>,
}

/// 计划游标与本次入队必须共同提交；过期槽只记录跳过，不追赶执行
#[derive(Debug, Clone)]
pub struct EnqueueScheduledEvaluation {
    pub plan_id: String,
    pub expected_revision: i64,
    pub scheduled_at: DateTime<Utc>,
    pub next_run_at: DateTime<Utc>,
    pub seed: EvaluationRunSeed,
    pub skip_reason: Option<String>,
}

/// Admin 提供保留阈值，仓储在数据库内有界筛选并清理终态记录
#[derive(Debug, Clone)]
pub struct EvaluationRetention {
    pub completed_before: DateTime<Utc>,
    pub keep_per_account_series: i64,
    pub max_output_bytes: i64,
    pub batch_size: u32,
}

impl EvaluationRetention {
    #[must_use]
    pub fn standard(now: DateTime<Utc>) -> Self {
        Self {
            completed_before: now - chrono::Duration::days(30),
            keep_per_account_series: 100,
            max_output_bytes: 512 * 1024 * 1024,
            batch_size: 100,
        }
    }
}
