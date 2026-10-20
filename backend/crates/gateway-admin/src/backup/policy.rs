//! 备份纯策略：Cron 推进与保留候选
//!
//! 本模块不持有任何端口或对象，是 `BackupService` 与 `BackupTask` 共用的纯函数集

use chrono::{DateTime, Duration, Utc};

use crate::model::backup::{BackupError, BackupRecord, BackupStatus, code};

/// 备份计划沿用统一日历语义，错误仍使用备份领域错误码
#[derive(Debug, Clone)]
pub struct BackupSchedule(crate::model::schedule::CronSchedule);

impl BackupSchedule {
    pub fn parse(
        cron_expression: &str,
        timezone: gateway_core::time::DeploymentTimeZone,
    ) -> Result<Self, BackupError> {
        crate::model::schedule::CronSchedule::parse(cron_expression, timezone)
            .map(Self)
            .map_err(|_| {
                BackupError::new(code::INVALID_CRON, "Cron 表达式必须是 5 段格式".to_owned())
            })
    }

    #[must_use]
    pub fn last_firing_at_or_before(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.0.last_firing_at_or_before(at)
    }

    #[must_use]
    pub fn next_after(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.0.next_after(at)
    }
}

/// 一条记录被纳入删除流程的原因
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionReason {
    /// 超过 `retention_days`
    Days,
    /// 超过 `retention_count`
    Count,
}

/// 保留决策
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionDecision {
    pub record_id: String,
    pub reason: RetentionReason,
}

/// 从按完成时间倒序的 completed 计划备份中决定纳入删除流程的记录
///
/// 自动保留规则：
/// - `retentionDays = 0` 时不按天数清理；`retentionCount = 0` 时不按份数清理
/// - 自动计划备份满足任一启用阈值即可进入删除
/// - 无论阈值如何，至少保留最近一个成功的计划备份（倒序下标 0）
/// - 手动备份不进入本规则；手工/计划记录各自的 `expires_at` 由到期扫描处理
#[must_use]
pub fn decide_retention(
    retention_days: u32,
    retention_count: u32,
    now: DateTime<Utc>,
    scheduled_completed_desc: &[BackupRecord],
) -> Vec<RetentionDecision> {
    let mut decisions = Vec::new();
    for (index, record) in scheduled_completed_desc.iter().enumerate() {
        if record.status != BackupStatus::Completed {
            continue;
        }
        // 最近一个成功的计划备份永远保留
        if index == 0 {
            continue;
        }
        let expired_by_days = retention_days > 0
            && record.completed_at.is_some_and(|completed_at| {
                completed_at <= now - Duration::days(i64::from(retention_days))
            });
        let exceeded_count =
            retention_count > 0 && index >= usize::try_from(retention_count).unwrap_or(usize::MAX);
        if expired_by_days || exceeded_count {
            decisions.push(RetentionDecision {
                record_id: record.id.clone(),
                reason: if expired_by_days {
                    RetentionReason::Days
                } else {
                    RetentionReason::Count
                },
            });
        }
    }
    decisions
}
