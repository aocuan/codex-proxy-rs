//! 正式测票方案、账号有效信号与执行状态，历史查询与优选共享同一判据

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::*;
use crate::model::AdminError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationMonitoringPolicy {
    pub valid_minutes: u32,
    pub stable_count: u8,
    pub changed_count: u8,
    #[serde(default)]
    pub remove_expired: bool,
}

impl Default for EvaluationMonitoringPolicy {
    fn default() -> Self {
        Self {
            valid_minutes: 30,
            stable_count: 2,
            changed_count: 2,
            remove_expired: false,
        }
    }
}

impl EvaluationMonitoringPolicy {
    pub fn validate(&self) -> Result<(), AdminError> {
        if !(1..=10_080).contains(&self.valid_minutes)
            || !(1..=10).contains(&self.stable_count)
            || !(1..=10).contains(&self.changed_count)
        {
            return Err(AdminError::invalid(
                "监测有效期须为 1 到 10080 分钟，连续次数须为 1 到 10 次",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn required_count(&self, outcome: TurnStateOutcome) -> usize {
        usize::from(if outcome == TurnStateOutcome::Accepted {
            self.stable_count
        } else {
            self.changed_count
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationPlanReference {
    pub id: String,
    pub revision: i64,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct EvaluationMonitoringPoint {
    pub plan_version: String,
    pub diagnostic: Option<TurnStateDiagnostic>,
    pub history: EvaluationHistoryItem,
    pub connection_revision: Option<String>,
    pub request_profile_revision: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EvaluationMonitoringData {
    pub observed_at: DateTime<Utc>,
    pub plan: Option<EvaluationPlan>,
    pub accounts: Vec<EvaluationAccountState>,
    pub points: Vec<EvaluationMonitoringPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationFreshness {
    Fresh,
    Expired,
    EnvironmentChanged,
    Unmeasured,
}

/// 当前凭据与检测身份下的明确授权证据，票据质量与授权有效性分别判断
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationAuthentication {
    pub status: &'static str,
    pub error_code: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub confirmation_started_at: DateTime<Utc>,
    pub connection_revision: String,
    pub item_id: String,
    pub confirmed: bool,
    pub consecutive: usize,
    pub required: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationAccountMonitoring {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<EvaluationAuthentication>,
    pub status: EvaluationAccountStatus,
    pub monitored: bool,
    pub freshness: EvaluationFreshness,
    pub signal: Option<TurnStateOutcome>,
    pub signal_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing)]
    pub latest_attempt: Option<EvaluationHistoryItem>,
    pub consecutive: usize,
    pub required: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationMonitoringSnapshot {
    pub observed_at: DateTime<Utc>,
    pub plan: Option<EvaluationPlan>,
    pub accounts: Vec<EvaluationAccountMonitoring>,
    pub history: Vec<EvaluationHistoryItem>,
}

impl EvaluationAccountMonitoring {
    #[must_use]
    pub fn derive(
        account: &EvaluationAccountState,
        supported: bool,
        monitored: bool,
        policy: &EvaluationMonitoringPolicy,
        points: &[&EvaluationMonitoringPoint],
        profile_revision: &str,
        now: DateTime<Utc>,
    ) -> Self {
        let latest = points.first().map(|point| point.history.clone());
        let same_environment = |point: &&EvaluationMonitoringPoint| {
            point.connection_revision.as_deref() == Some(account.connection_revision.as_str())
                && point.request_profile_revision.as_deref() == Some(profile_revision)
        };
        let valid = |point: &&EvaluationMonitoringPoint| {
            point.history.status == EvaluationItemStatus::Succeeded
                && matches!(
                    point.history.turn_state,
                    Some(TurnStateOutcome::Accepted | TurnStateOutcome::Refreshed)
                )
        };
        let signal = points.iter().copied().filter(same_environment).find(valid);
        let signal_at = signal.and_then(|point| point.history.completed_at);
        let expires_at =
            signal_at.map(|at| at + chrono::Duration::minutes(i64::from(policy.valid_minutes)));
        let freshness = match expires_at {
            Some(at) if at > now => EvaluationFreshness::Fresh,
            Some(_) => EvaluationFreshness::Expired,
            None if points.iter().copied().any(|point| valid(&point)) => {
                EvaluationFreshness::EnvironmentChanged
            }
            None => EvaluationFreshness::Unmeasured,
        };
        let outcome = signal.and_then(|point| point.history.turn_state);
        // 取消和跳过没有新观测，不消耗连续确认次数；失败或未判定会中断连续确认
        let consecutive = points
            .iter()
            .copied()
            .filter(|point| {
                !matches!(
                    point.history.status,
                    EvaluationItemStatus::Queued
                        | EvaluationItemStatus::Running
                        | EvaluationItemStatus::Skipped
                        | EvaluationItemStatus::Cancelled
                )
            })
            .take_while(|point| {
                same_environment(point)
                    && valid(point)
                    && point.history.turn_state == outcome
                    && point.history.completed_at.is_some_and(|at| {
                        at + chrono::Duration::minutes(i64::from(policy.valid_minutes)) > now
                    })
            })
            .count();
        let status = if !supported {
            EvaluationAccountStatus::NotApplicable
        } else if !monitored {
            EvaluationAccountStatus::NoRecord
        } else if matches!(
            freshness,
            EvaluationFreshness::Expired | EvaluationFreshness::EnvironmentChanged
        ) {
            EvaluationAccountStatus::Stale
        } else if latest.as_ref().is_some_and(|point| {
            matches!(
                point.status,
                EvaluationItemStatus::Failed | EvaluationItemStatus::Interrupted
            )
        }) {
            EvaluationAccountStatus::Failed
        } else if latest.as_ref().is_some_and(|point| {
            point.status == EvaluationItemStatus::Succeeded
                && !matches!(
                    point.turn_state,
                    Some(TurnStateOutcome::Accepted | TurnStateOutcome::Refreshed)
                )
        }) {
            EvaluationAccountStatus::Unknown
        } else if let Some(outcome) = outcome {
            EvaluationAccountStatus::from_result(
                true,
                Some(EvaluationItemStatus::Succeeded),
                Some(outcome),
            )
        } else {
            EvaluationAccountStatus::from_result(true, latest.as_ref().map(|p| p.status), None)
        };
        Self {
            account_id: account.id.clone(),
            authentication: monitored
                .then(|| authentication_evidence(account, points, profile_revision, policy, now))
                .flatten(),
            status,
            monitored,
            freshness,
            signal: outcome,
            signal_at,
            expires_at,
            latest_attempt: latest,
            consecutive,
            required: outcome.map_or(usize::from(policy.stable_count), |value| {
                policy.required_count(value)
            }),
        }
    }
}

fn authentication_evidence(
    account: &EvaluationAccountState,
    points: &[&EvaluationMonitoringPoint],
    profile: &str,
    policy: &EvaluationMonitoringPolicy,
    now: DateTime<Utc>,
) -> Option<EvaluationAuthentication> {
    let rejected = |point: &EvaluationMonitoringPoint| {
        point.history.status == EvaluationItemStatus::Failed
            && point
                .diagnostic
                .as_ref()
                .is_some_and(|d| d.unavailable == Some(TurnStateUnavailable::Authorization))
    };
    // 网络、过载、取消与排队没有授权结论，不清除已确认的拒绝，也不算成功
    let mut decisive = points.iter().copied().filter(|point| {
        point.connection_revision.as_deref() == Some(account.connection_revision.as_str())
            && point.request_profile_revision.as_deref() == Some(profile)
            && point.history.completed_at.is_some_and(|at| {
                at <= now && at + chrono::Duration::minutes(i64::from(policy.valid_minutes)) > now
            })
            && (point.history.status == EvaluationItemStatus::Succeeded || rejected(point))
    });
    let latest = decisive.next()?;
    let failed = rejected(latest);
    let required = if failed {
        usize::from(policy.changed_count)
    } else {
        1
    };
    let mut consecutive = 1;
    let mut confirmation_started_at = latest.history.completed_at?;
    if failed {
        for point in decisive.take_while(|point| rejected(point)) {
            consecutive += 1;
            if consecutive <= required {
                confirmation_started_at = point.history.completed_at?;
            }
        }
    }
    Some(EvaluationAuthentication {
        status: if failed { "rejected" } else { "valid" },
        error_code: if failed {
            latest
                .diagnostic
                .as_ref()
                .and_then(|d| d.error_code.as_ref())
                .map(|value| value.chars().take(100).collect())
        } else {
            None
        },
        observed_at: latest.history.completed_at?,
        confirmation_started_at,
        connection_revision: account.connection_revision.clone(),
        item_id: latest.history.id.clone(),
        confirmed: consecutive >= required,
        consecutive,
        required,
    })
}

impl EvaluationPlan {
    #[must_use]
    pub fn monitoring_reference(&self) -> EvaluationPlanReference {
        use sha2::{Digest as _, Sha256};
        let settings = &self.settings;
        let version = serde_json::json!([
            settings.parameters.kind,
            (settings.parameters.kind != EvaluationKind::Turnstate)
                .then_some(&settings.parameters.client_key_id),
            settings.parameters.model,
            settings.parameters.reasoning_effort,
            settings.parameters.timeout_seconds,
            settings.account_selection,
            settings.monitoring,
            settings.account_preference,
            EvaluationKind::Turnstate.prompt_version(),
            EvaluationKind::Turnstate.grader_version(),
            EvaluationKind::Turnstate.request_profile_version()
        ])
        .to_string();
        EvaluationPlanReference {
            id: self.id.clone(),
            revision: self.revision,
            version: hex::encode(Sha256::digest(version.as_bytes())),
        }
    }
}

/// 正式方案回收当前目标组中的不可用账号和需要回收的过期结果
#[derive(Debug, Clone)]
pub struct EvaluationPreferenceCleanup {
    pub plan: EvaluationPlanReference,
    pub accounts: Vec<EvaluationPreferenceCleanupTarget>,
    pub expected_revision: crate::model::Revision,
}

#[derive(Debug, Clone)]
pub struct EvaluationPreferenceCleanupTarget {
    pub account_id: String,
    pub reason: EvaluationPreferenceCleanupReason,
}

#[derive(Debug, Clone)]
pub enum EvaluationPreferenceCleanupReason {
    AccountUnavailable,
    /// 提交时用观测身份排除快照之后已有新结果的账号
    Expired {
        last_signal_id: Option<String>,
    },
}
