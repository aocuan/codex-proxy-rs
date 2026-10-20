//! 单副本测评 Daemon：有界并发、定时推进、取消排空与启动保守恢复

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures::{StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
use gateway_core::{
    engine::execution::ExecutionService,
    lifecycle::CancellationToken,
    task::{DaemonTask, WorkerTaskError},
    time::DeploymentTimeZone,
};

use super::runner::{EvaluationRunner, store_error};
use crate::{
    model::{MutationActor, MutationContext, evaluation::*},
    ports::{
        evaluation::EvaluationRepository,
        store::{AdminStoreErrorKind, ObservabilityStore},
    },
};

pub struct EvaluationTask {
    runner: EvaluationRunner,
    version: String,
    timezone: DeploymentTimeZone,
}

impl EvaluationTask {
    #[must_use]
    pub fn new(
        repository: Arc<dyn EvaluationRepository>,
        execution: Arc<dyn ExecutionService>,
        observability: Arc<dyn ObservabilityStore>,
        version: String,
        timezone: DeploymentTimeZone,
    ) -> Self {
        Self {
            runner: EvaluationRunner {
                typesafe: None,
                repository,
                execution,
                observability,
            },
            version,
            timezone,
        }
    }

    #[must_use]
    pub fn with_typesafe(
        mut self,
        client: Option<Arc<dyn crate::ports::evaluation::TypeSafeJudgeClient>>,
    ) -> Self {
        self.runner.typesafe = client;
        self
    }

    /// 供 Worker 与集成验证共同使用，不改变已创建批次的参数
    pub async fn advance_schedules(&self, now: DateTime<Utc>) -> Result<(), WorkerTaskError> {
        for mut plan in self
            .runner
            .repository
            .list_plans()
            .await
            .map_err(store_error)?
        {
            if !plan.settings.enabled {
                continue;
            }
            if plan.settings.interval_minutes.is_none() && plan.settings.timezone != self.timezone {
                // 部署切换时区只建立新的未来游标，不执行旧时区到期槽
                plan.settings.timezone = self.timezone;
                let next = plan
                    .settings
                    .next_occurrence_after(now, now)
                    .map_err(|error| {
                        WorkerTaskError::safe("测评计划配置无效").with_source(error)
                    })?;
                let changed = self
                    .runner
                    .repository
                    .save_plan(
                        SaveEvaluationPlan {
                            id: plan.id,
                            expected_revision: Some(plan.revision),
                            settings: plan.settings,
                            next_run_at: Some(next),
                        },
                        &MutationContext {
                            actor: MutationActor::System,
                            request_id: "evaluation_timezone_rebase".to_owned(),
                        },
                    )
                    .await;
                match changed {
                    Ok(_) => {}
                    Err(error) if error.kind() == AdminStoreErrorKind::StaleRevision => {}
                    Err(error) => return Err(store_error(error)),
                }
                continue;
            }
            let Some(at) = plan.next_run_at.filter(|at| *at <= now) else {
                continue;
            };
            let next = plan
                .settings
                .next_occurrence_after(at, now)
                .map_err(|error| WorkerTaskError::safe("测评计划配置无效").with_source(error))?;
            let skip_reason = if now.signed_duration_since(at) >= chrono::Duration::minutes(1) {
                Some("错过执行时间，已跳过历史时段".to_owned())
            } else {
                None
            };
            self.runner
                .repository
                .enqueue_scheduled(EnqueueScheduledEvaluation {
                    plan_id: plan.id,
                    expected_revision: plan.revision,
                    scheduled_at: at,
                    next_run_at: next,
                    seed: EvaluationRunSeed::new(plan.settings.parameters, &self.version, now),
                    skip_reason,
                })
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    async fn run_daemon(&self, parent: CancellationToken) -> Result<(), WorkerTaskError> {
        self.runner
            .repository
            .recover_running(Utc::now())
            .await
            .map_err(store_error)?;
        let cancellation = parent.child_token();
        let mut active: FuturesUnordered<BoxFuture<'_, Result<(), WorkerTaskError>>> =
            FuturesUnordered::new();
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_maintenance = None;
        let mut failure = None;
        loop {
            if parent.is_cancelled() {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() && active.is_empty() {
                break;
            }
            tokio::select! {
                _ = parent.cancelled(), if !cancellation.is_cancelled() => cancellation.cancel(),
                result = active.next(), if !active.is_empty() => {
                    if let Some(Err(error)) = result {
                        failure = Some(error);
                        cancellation.cancel();
                    }
                }
                _ = interval.tick(), if !cancellation.is_cancelled() => {
                    let now = Utc::now();
                    let maintenance = async {
                        if last_maintenance.is_none_or(|last| now.signed_duration_since(last) >= chrono::Duration::seconds(30)) {
                            self.advance_schedules(now).await?;
                            self.runner.repository.prune_history(EvaluationRetention::standard(now)).await.map_err(store_error)?;
                            last_maintenance = Some(now);
                        }
                        self.fill_queue(&mut active, &cancellation, now).await?;
                        Ok::<_, WorkerTaskError>(())
                    }.await;
                    if let Err(error) = maintenance {
                        failure = Some(error);
                        cancellation.cancel();
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn fill_queue<'a>(
        &'a self,
        active: &mut FuturesUnordered<BoxFuture<'a, Result<(), WorkerTaskError>>>,
        cancellation: &CancellationToken,
        now: DateTime<Utc>,
    ) -> Result<(), WorkerTaskError> {
        while active.len() < MAX_PARALLEL_REQUESTS as usize && !cancellation.is_cancelled() {
            if let Some(claim) = self
                .runner
                .repository
                .claim_judgment(now)
                .await
                .map_err(store_error)?
            {
                active.push(Box::pin(
                    self.runner.run_judgment(claim, cancellation.clone()),
                ));
                continue;
            }
            let Some(claim) = self
                .runner
                .repository
                .claim_next(now)
                .await
                .map_err(store_error)?
            else {
                break;
            };
            active.push(Box::pin(self.runner.run(claim, cancellation.clone())));
        }
        Ok(())
    }
}

impl DaemonTask for EvaluationTask {
    fn run(&self, cancellation: CancellationToken) -> BoxFuture<'_, Result<(), WorkerTaskError>> {
        Box::pin(self.run_daemon(cancellation))
    }
}
