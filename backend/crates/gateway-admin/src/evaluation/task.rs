//! 单副本测评 Daemon：有界并发、定时推进、取消排空与启动保守恢复

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures::{StreamExt as _, future::BoxFuture, stream::FuturesUnordered};
use gateway_core::{
    engine::execution::ExecutionService,
    lifecycle::CancellationToken,
    task::{
        DaemonRestartPolicy, DaemonTask, WorkerContribution, WorkerDaemonContext, WorkerId,
        WorkerKind, WorkerRegistration, WorkerRunnable, WorkerTaskError,
    },
    time::DeploymentTimeZone,
};

use super::runner::{EvaluationRunner, store_error};
use crate::{
    model::{AdminError, MutationActor, MutationContext, evaluation::*},
    ports::{
        evaluation::EvaluationRepository,
        store::{AdminStoreErrorKind, ObservabilityStore},
    },
};

pub struct EvaluationTask {
    runner: EvaluationRunner,
    version: String,
    timezone: DeploymentTimeZone,
    snapshot: gateway_core::runtime::RuntimeSnapshotHandle,
}

impl EvaluationTask {
    pub(crate) fn into_worker(self) -> Result<WorkerContribution, AdminError> {
        let id = WorkerId::try_new(WorkerKind::ModelEvaluation, "admin")
            .map_err(|_| AdminError::internal("测评 Worker ID 不合法"))?;
        let restart = DaemonRestartPolicy::try_new(Duration::from_secs(1), Duration::from_secs(60))
            .map_err(|_| AdminError::internal("测评 Worker 重启策略不合法"))?;
        let registration = WorkerRegistration::try_new(
            id,
            WorkerRunnable::Daemon {
                restart,
                task: Box::new(self),
            },
        )
        .map_err(|_| AdminError::internal("测评 Worker 注册失败"))?;
        Ok(WorkerContribution::Registration(registration))
    }

    #[must_use]
    pub fn new(
        repository: Arc<dyn EvaluationRepository>,
        execution: Arc<dyn ExecutionService>,
        observability: Arc<dyn ObservabilityStore>,
        version: String,
        timezone: DeploymentTimeZone,
        snapshot: gateway_core::runtime::RuntimeSnapshotHandle,
    ) -> Self {
        Self {
            runner: EvaluationRunner {
                providers: None,
                publisher: None,
                snapshot: snapshot.clone(),
                typesafe: None,
                repository,
                execution,
                observability,
            },
            version,
            timezone,
            snapshot,
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

    #[must_use]
    pub fn with_turnstate(
        mut self,
        providers: crate::ports::provider::ProviderAdminRegistry,
    ) -> Self {
        self.runner.providers = Some(providers);
        self
    }

    #[must_use]
    pub fn with_snapshot_control(
        mut self,
        publisher: Arc<dyn gateway_core::runtime::SnapshotControl>,
    ) -> Self {
        self.runner.publisher = Some(publisher);
        self
    }

    /// 供 Worker 与集成验证共同使用，不改变已创建批次的参数
    pub async fn advance_schedules(&self, now: DateTime<Utc>) -> Result<(), WorkerTaskError> {
        let accounts = self
            .runner
            .repository
            .account_states()
            .await
            .map_err(store_error)?;
        let plans = self
            .runner
            .repository
            .list_plans()
            .await
            .map_err(store_error)?;
        let mut cleanup = None;
        for mut plan in plans {
            if plan.settings.account_selection.is_fixed() {
                let previous = plan.settings.parameters.account_ids.len();
                let kind = plan.settings.parameters.kind;
                plan.settings.parameters.account_ids.retain(|id| {
                    accounts.iter().any(|a| {
                        &a.id == id
                            && super::accounts::supports(a, kind, self.runner.providers.as_ref())
                    })
                });
                if previous != plan.settings.parameters.account_ids.len() {
                    if plan.settings.parameters.account_ids.is_empty() {
                        plan.settings.enabled = false;
                        plan.next_run_at = None;
                    }
                    match self
                        .runner
                        .repository
                        .save_plan(
                            SaveEvaluationPlan {
                                id: plan.id.clone(),
                                expected_revision: Some(plan.revision),
                                settings: plan.settings.clone(),
                                next_run_at: plan.next_run_at,
                            },
                            &MutationContext {
                                actor: MutationActor::System,
                                request_id: "evaluation_account_reconcile".to_owned(),
                            },
                        )
                        .await
                    {
                        Ok(saved) => plan = saved,
                        Err(error) if error.kind() == AdminStoreErrorKind::StaleRevision => {
                            continue;
                        }
                        Err(error) => return Err(store_error(error)),
                    }
                }
            }
            if !plan.settings.enabled {
                continue;
            }
            if plan.settings.monitoring.is_some() && plan.settings.account_preference.is_some() {
                cleanup = Some(plan.clone());
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
            let account_ids = super::accounts::resolve(
                &plan.settings,
                &accounts,
                &self.snapshot,
                self.runner.providers.as_ref(),
            )
            .map_err(|error| WorkerTaskError::safe("无法解析计划账号范围").with_source(error))?;
            let mut parameters = plan.settings.parameters.clone();
            parameters.account_ids = account_ids;
            let maximum = if parameters.kind == EvaluationKind::Turnstate {
                MAX_SCHEDULED_PROBE_ACCOUNTS
            } else {
                MAX_ACCOUNTS_PER_RUN
            };
            if parameters.account_ids.len() > maximum {
                tracing::warn!(plan_id=%plan.id, available=parameters.account_ids.len(), maximum, "计划账号范围超过单轮上限，本轮不执行");
                // 计划页仍展示完整可用数与超限提示，不截断账号导致固定一部分永远漏测
                parameters.account_ids.clear();
            }
            let mut seed = EvaluationRunSeed::new(parameters, &self.version, now);
            seed.profile.account_selection = plan.settings.account_selection.clone();
            seed.profile.monitoring = plan
                .settings
                .monitoring
                .as_ref()
                .map(|_| plan.monitoring_reference());
            seed.profile.account_preference = plan
                .settings
                .account_preference
                .as_ref()
                .filter(|_| plan.settings.monitoring.is_some())
                .map(|rule| EvaluationPreferenceSnapshot {
                    group_id: rule.group_id.clone(),
                    plan_revision: plan.revision,
                });
            self.runner
                .repository
                .enqueue_scheduled(EnqueueScheduledEvaluation {
                    plan_id: plan.id,
                    expected_revision: plan.revision,
                    scheduled_at: at,
                    next_run_at: next,
                    seed,
                    skip_reason,
                })
                .await
                .map_err(store_error)?;
        }
        if let Some(plan) = &cleanup {
            self.cleanup_preference(plan, &accounts).await?;
        }
        Ok(())
    }

    async fn cleanup_preference(
        &self,
        plan: &EvaluationPlan,
        accounts: &[EvaluationAccountState],
    ) -> Result<(), WorkerTaskError> {
        let (Some(rule), Some(policy)) =
            (&plan.settings.account_preference, &plan.settings.monitoring)
        else {
            return Ok(());
        };
        let mut targets = std::collections::BTreeMap::new();
        for account in accounts.iter().filter(|account| {
            !preference_account_available(account.enabled, account.credential_state)
                && account.group_ids.contains(&rule.group_id)
                && plan
                    .settings
                    .includes_account(&account.id, &account.group_ids)
                && super::accounts::supports(
                    account,
                    EvaluationKind::Turnstate,
                    self.runner.providers.as_ref(),
                )
        }) {
            targets.insert(
                account.id.clone(),
                EvaluationPreferenceCleanupReason::AccountUnavailable,
            );
        }
        if targets.is_empty() && !policy.remove_expired {
            return Ok(());
        }
        let Some(publisher) = &self.runner.publisher else {
            return Err(WorkerTaskError::safe("账号优选的配置发布服务不可用"));
        };
        let Ok(snapshot) = self.snapshot.acquire() else {
            // 配置暂未发布时保留成员，下轮维护使用新快照
            return Ok(());
        };
        let revision = snapshot.revision();
        let reference = plan.monitoring_reference();
        if policy.remove_expired {
            let data = self
                .runner
                .repository
                .monitoring_data()
                .await
                .map_err(store_error)?;
            if data
                .plan
                .as_ref()
                .is_some_and(|current| current.monitoring_reference() == reference)
            {
                let mut last_signals = std::collections::BTreeMap::<String, String>::new();
                for point in &data.points {
                    if point.history.status == EvaluationItemStatus::Succeeded
                        && matches!(
                            point.history.turn_state,
                            Some(TurnStateOutcome::Accepted | TurnStateOutcome::Refreshed)
                        )
                    {
                        last_signals
                            .entry(point.history.account_id.clone())
                            .or_insert_with(|| point.history.id.clone());
                    }
                }
                let members = data
                    .accounts
                    .iter()
                    .filter(|account| account.group_ids.contains(&rule.group_id))
                    .map(|account| account.id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                let view = super::monitoring::project(
                    data,
                    &self.snapshot,
                    self.runner.providers.as_ref(),
                )
                .map_err(|error| WorkerTaskError::safe("监测到期检查失败").with_source(error))?;
                for account in view.accounts.into_iter().filter(|account| {
                    account.monitored
                        && members.contains(&account.account_id)
                        && matches!(
                            account.freshness,
                            EvaluationFreshness::Expired | EvaluationFreshness::EnvironmentChanged
                        )
                }) {
                    let last_signal_id = last_signals.remove(&account.account_id);
                    targets
                        .entry(account.account_id)
                        .or_insert(EvaluationPreferenceCleanupReason::Expired { last_signal_id });
                }
            }
        }
        if targets.is_empty() {
            return Ok(());
        }
        let committed = self
            .runner
            .repository
            .cleanup_preference(EvaluationPreferenceCleanup {
                plan: reference,
                accounts: targets
                    .into_iter()
                    .map(|(account_id, reason)| EvaluationPreferenceCleanupTarget {
                        account_id,
                        reason,
                    })
                    .collect(),
                expected_revision: crate::model::Revision::new(revision.get())
                    .map_err(|error| WorkerTaskError::safe("配置版本无效").with_source(error))?,
            })
            .await
            .map_err(store_error)?;
        if let Some(revision) = committed {
            let revision = gateway_core::routing::ConfigRevision::new(revision.get())
                .map_err(|error| WorkerTaskError::safe("配置版本无效").with_source(error))?;
            publisher.publish_committed(revision).await;
        }
        Ok(())
    }

    async fn run_daemon(&self, context: WorkerDaemonContext) -> Result<(), WorkerTaskError> {
        let parent = context.cancellation();
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
                    let maintenance_due = last_maintenance.is_none_or(|last| now.signed_duration_since(last) >= chrono::Duration::seconds(30));
                    let maintenance = async {
                        self.runner.repository.reconcile_accounts().await.map_err(store_error)?;
                        if maintenance_due {
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
                    } else if maintenance_due {
                        // 计划、清理与领取检查均成功后再报告健康，空队列也有真实仓储检查
                        context.succeeded();
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
    fn run(&self, context: WorkerDaemonContext) -> BoxFuture<'_, Result<(), WorkerTaskError>> {
        Box::pin(self.run_daemon(context))
    }
}
