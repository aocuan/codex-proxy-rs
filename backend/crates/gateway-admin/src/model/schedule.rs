//! 备份与测评共用的日历调度；UTC 游标、夏令时缺口跳过、重复时刻只取较早一次

use std::str::FromStr;

use chrono::{DateTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use cron::Schedule;

/// 向历史回溯的最大范围（分钟）
/// 覆盖 Feb 29 + 特定星期等罕见组合的最长间隔
const MAX_BACKDATE_MINUTES: i64 = 40 * 366 * 24 * 60;

/// 已解析的日历计划
#[derive(Debug, Clone)]
pub struct CronSchedule {
    schedule: Schedule,
    timezone: Tz,
}

impl CronSchedule {
    /// 按部署时区解析分钟级 Cron，具体输入字段数由调用方校验
    ///
    /// # Errors
    ///
    /// Cron 非法时返回解析错误
    pub fn parse(
        cron_expression: &str,
        timezone: gateway_core::time::DeploymentTimeZone,
    ) -> Result<Self, cron::error::Error> {
        let expression = format!("0 {cron_expression}");
        let schedule = Schedule::from_str(&expression)?;
        Ok(Self {
            schedule,
            timezone: timezone.iana(),
        })
    }

    /// 返回最接近 `at`（含 `at` 所在分钟）的最近一次触发时间
    ///
    /// 若过去 `MAX_BACKDATE_MINUTES` 内没有任何触发（极罕见的稀疏计划或超长停机），
    /// 返回 `None`，调用方不应为历史点补任务
    #[must_use]
    pub fn last_firing_at_or_before(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let high_min = at.timestamp().div_euclid(60);
        let low_min = high_min.saturating_sub(MAX_BACKDATE_MINUTES);

        // 检查窗口内是否存在触发；不存在则直接返回 None
        let low_instant = instant(low_min, self.timezone)?;
        if self
            .next_after(low_instant.to_utc())
            .is_none_or(|firing| firing.timestamp() > high_min * 60 + 59)
        {
            return None;
        }

        // 二分查找最小的 u（分钟）使 after(u) > high；prev = after(u - 1 分钟)
        let mut lo = low_min;
        let mut hi = high_min;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self
                .firing_after(mid)
                .is_some_and(|firing| firing > high_min)
            {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        self.firing_after(lo - 1)
            .filter(|&firing| firing <= high_min)
            .and_then(|firing| Utc.timestamp_opt(firing * 60, 0).single())
    }

    /// 返回第一个严格晚于 `at` 的触发时间
    #[must_use]
    pub fn next_after(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let local = at.with_timezone(&self.timezone).naive_local().and_utc();
        self.schedule.after(&local).find_map(|firing| {
            let instant = self
                .timezone
                .from_local_datetime(&firing.naive_utc())
                .earliest()?
                .to_utc();
            (instant > at).then_some(instant)
        })
    }

    /// `after(u 分钟)` 返回的第一个触发时间（分钟索引），用于二分搜索
    fn firing_after(&self, minute_index: i64) -> Option<i64> {
        let instant = instant(minute_index, self.timezone)?.to_utc();
        self.next_after(instant)
            .map(|firing| firing.timestamp().div_euclid(60))
    }
}

/// 构造指定分钟索引（UTC epoch 分钟）在目标时区下的 `DateTime`
fn instant(minute_index: i64, timezone: Tz) -> Option<DateTime<Tz>> {
    timezone.timestamp_opt(minute_index * 60, 0).single()
}
