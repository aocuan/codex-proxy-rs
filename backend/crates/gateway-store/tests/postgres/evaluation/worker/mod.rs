//! 使用真实测评仓储验证 Worker 的接收、判题、停止、持久输出与收尾顺序

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use chrono::Utc;
use futures::future::BoxFuture;
use gateway_admin::{
    evaluation::{EvaluationService, EvaluationTask},
    model::{MutationActor, MutationContext, evaluation::*},
    ports::evaluation::EvaluationRepository,
};
use gateway_core::{
    engine::{CoordinatedEvent, EngineError, ModelRequestId, execution::*},
    error::GatewayError,
    event::{
        FinishReason, GatewayEvent, ProviderEvent, ProviderResponseHeader, ResponseMeta, TextDelta,
    },
    lifecycle::CancellationToken,
    operation::Operation,
    routing::PublicModelId,
    task::{DaemonTask, WorkerDaemonContext},
};
use gateway_store::postgres::PgEvaluationRepository;

use super::super::{TestDatabase, admin_observability_store};

mod accounts;
mod turnstate;
mod typesafe;

#[derive(Clone, Copy)]
enum Reply {
    Answer,
    Pending,
    Oversized,
    CommitFailure,
    JudgeInvalid,
    JudgePending,
}

struct Execution {
    pool: sqlx::PgPool,
    repository: Arc<PgEvaluationRepository>,
    reply: Reply,
    accounts: Mutex<Vec<String>>,
    commits: Arc<AtomicUsize>,
    cleanups: Arc<AtomicUsize>,
}

impl ExecutionService for Execution {
    fn authenticate(&self, _: &str) -> Result<AuthenticatedClient, ClientAuthenticationError> {
        panic!("测评不能读取明文 Client Key")
    }
    fn public_models(&self, _: &AuthenticatedClient) -> Vec<PublicModelId> {
        unreachable!()
    }
    fn contains_public_model(&self, _: &AuthenticatedClient, _: &PublicModelId) -> bool {
        unreachable!()
    }
    fn start(&self, _: StartExecution) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        panic!("测评必须固定账号")
    }
    fn start_provider_endpoint(
        &self,
        _: StartProviderExecution,
    ) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        unreachable!()
    }
    fn start_account_execution(
        &self,
        request: StartAccountExecution,
    ) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        Box::pin(async move {
            assert_eq!(request.client_key_id.as_str(), "key_evaluation");
            assert_eq!(request.metadata.transport, ClientTransport::InternalAdmin);
            assert!(request.metadata.client_ip.is_none());
            assert!(request.deadline_at > SystemTime::now());
            assert!(request.deadline_at <= SystemTime::now() + Duration::from_secs(600));
            let Operation::Generate(operation) = request.operation else {
                panic!("generate")
            };
            let body = operation.protocol_payload().body();
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert_eq!(body["tools"], serde_json::json!([]));
            assert_eq!(body["reasoning"]["effort"], "high");
            assert!(body.get("max_output_tokens").is_none());
            let judge = request.account_id.as_str() == "acct_judge";
            if judge {
                assert_eq!(body["input"][0]["role"], "system");
                assert!(
                    body["input"][1]["content"]
                        .as_str()
                        .unwrap()
                        .contains("candidateOutput")
                );
            }
            self.accounts
                .lock()
                .unwrap()
                .push(request.account_id.as_str().to_owned());
            let request_id =
                ModelRequestId::new(format!("req_{}", uuid::Uuid::now_v7().simple())).unwrap();
            Ok(StartedExecution {
                request_id: request_id.clone(),
                created_at: SystemTime::now(),
                stream: true,
                session: Box::new(Session {
                    pool: self.pool.clone(),
                    judge,
                    request_id: request_id.as_str().to_owned(),
                    repository: self.repository.clone(),
                    reply: self.reply,
                    wrong: request.account_id.as_str().ends_with("wrong"),
                    cancellation: request.cancellation,
                    commits: self.commits.clone(),
                    cleanups: self.cleanups.clone(),
                    finalized: false,
                }),
            })
        })
    }
}

struct Session {
    pool: sqlx::PgPool,
    judge: bool,
    request_id: String,
    repository: Arc<PgEvaluationRepository>,
    reply: Reply,
    wrong: bool,
    cancellation: CancellationToken,
    commits: Arc<AtomicUsize>,
    cleanups: Arc<AtomicUsize>,
    finalized: bool,
}

impl ExecutionSession for Session {
    fn next_event(&mut self) -> BoxFuture<'_, Result<Option<CoordinatedEvent>, EngineError>> {
        unreachable!()
    }
    fn collect_uncommitted(&mut self) -> BoxFuture<'_, Result<Vec<ProviderEvent>, EngineError>> {
        panic!("测评必须使用有界收集")
    }
    fn collect_uncommitted_into<'a>(
        &'a mut self,
        collector: &'a mut dyn UncommittedEventCollector,
    ) -> BoxFuture<'a, Result<(), EngineError>> {
        Box::pin(async move {
            collector.clear();
            let text = match self.reply {
                Reply::JudgeInvalid if self.judge => "不是 JSON".to_owned(),
                Reply::JudgePending if self.judge => "部分判题".to_owned(),
                _ if self.judge => r#"{"pass":false,"reason":"模拟独立评审未通过"}"#.to_owned(),
                Reply::Oversized => "x".repeat(MAX_RESPONSE_BYTES + 1),
                Reply::Pending => "部分回答".to_owned(),
                _ if self.wrong => "正文里提到了 21，但它不满足条件\n最终答案：29".to_owned(),
                _ => "9 + 12 = 21\n最终答案：21".to_owned(),
            };
            collector
                .accept(vec![
                    GatewayEvent::TextDelta(TextDelta {
                        content_index: 0,
                        text,
                    })
                    .into(),
                ])
                .map_err(|_| EngineError::CollectionLimitExceeded)?;
            if matches!(self.reply, Reply::Pending)
                || (self.judge && matches!(self.reply, Reply::JudgePending))
            {
                self.cancellation.cancelled().await;
                return Err(EngineError::Cancelled);
            }
            collector
                .accept(vec![
                    GatewayEvent::Completed(
                        ResponseMeta::new("resp_test", "test-model")
                            .with_finish_reason(FinishReason::Stop),
                    )
                    .into(),
                ])
                .unwrap();
            Ok(())
        })
    }
    fn response_headers(&self) -> &[ProviderResponseHeader] {
        &[]
    }
    fn commit_downstream(&mut self, _: Option<u16>) -> BoxFuture<'_, Result<(), EngineError>> {
        Box::pin(async move {
            let runs = self.repository.list_runs(100, 0).await.unwrap();
            let mut saved = if self.judge {
                sqlx::query_scalar::<_, bool>("select exists(select 1 from evaluation_judgments where request_id = $1 and response_text is not null)")
                    .bind(&self.request_id).fetch_one(&self.pool).await.unwrap()
            } else {
                false
            };
            for run in runs {
                let detail = self.repository.load_run(&run.id).await.unwrap().unwrap();
                if let Some(item) = detail
                    .items
                    .iter()
                    .find(|item| item.request_id.as_deref() == Some(&self.request_id))
                {
                    saved = self
                        .repository
                        .load_output(&item.id)
                        .await
                        .unwrap()
                        .is_some();
                }
            }
            assert!(saved, "提交消费事实之前必须已持久化完整回答");
            if matches!(self.reply, Reply::CommitFailure) {
                return Err(EngineError::InvalidDeliveryState);
            }
            self.commits.fetch_add(1, Ordering::SeqCst);
            self.cleanups.fetch_add(1, Ordering::SeqCst);
            self.finalized = true;
            Ok(())
        })
    }
    fn record_client_status(&mut self, _: u16) -> BoxFuture<'_, Result<(), EngineError>> {
        Box::pin(async { Ok(()) })
    }
    fn is_finalized(&self) -> bool {
        self.finalized
    }
    fn cancel(&self) {
        self.cancellation.cancel();
    }
    fn detach_finalize(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            assert!(self.cancellation.is_cancelled());
            self.cleanups.fetch_add(1, Ordering::SeqCst);
        })
    }
}

fn parameters(accounts: &[&str], repetitions: u8) -> EvaluationParameters {
    EvaluationParameters {
        definition: None,
        kind: EvaluationKind::Candy,
        client_key_id: "key_evaluation".to_owned(),
        account_ids: accounts.iter().map(|id| (*id).to_owned()).collect(),
        model: PublicModelId::new("test-model").unwrap(),
        reasoning_effort: Some("high".to_owned()),
        repetitions,
        timeout_seconds: 600,
    }
}
fn context() -> MutationContext {
    MutationContext {
        actor: MutationActor::System,
        request_id: "eval-worker-test".to_owned(),
    }
}
async fn setup(
    db: &TestDatabase,
    reply: Reply,
) -> (
    Arc<PgEvaluationRepository>,
    Arc<Execution>,
    EvaluationTask,
    EvaluationService,
) {
    sqlx::query("insert into provider_accounts (id,provider_kind,name,authentication_kind,provider_credentials_json,has_refresh_token,credential_observed_at,created_at,updated_at)
        select id,'openai',id,'api_key','{}'::jsonb,false,now(),now(),now() from unnest($1::text[]) id")
        .bind(vec!["acct_right", "acct_wrong", "acct_pending", "acct_judge", "acct_target"])
        .execute(&db.pool).await.unwrap();
    let repository = Arc::new(PgEvaluationRepository::new(db.pool.clone()));
    let execution = Arc::new(Execution {
        pool: db.pool.clone(),
        repository: repository.clone(),
        reply,
        accounts: Mutex::new(Vec::new()),
        commits: Arc::default(),
        cleanups: Arc::default(),
    });
    let provider = gateway_core::identity::ProviderKind::new("openai").unwrap();
    let directory = Arc::new(gateway_core::account::scope::RuntimeAccountDirectory::new(
        [
            "acct_right",
            "acct_wrong",
            "acct_pending",
            "acct_judge",
            "acct_target",
        ]
        .into_iter()
        .map(|id| {
            (
                gateway_core::account::ProviderAccountId::new(id).unwrap(),
                gateway_core::account::scope::RuntimeAccount::new(
                    provider.clone(),
                    Default::default(),
                ),
            )
        })
        .collect(),
    ));
    let policy = gateway_core::policy::ClientPolicy::new(
        gateway_core::policy::ClientApiKeyId::new("key_evaluation").unwrap(),
        gateway_core::policy::PlaintextClientApiKey::new("test-only").unwrap(),
        Arc::new(gateway_core::account::scope::FrozenAccountScope::new(
            directory,
            gateway_core::account::scope::ClientRoutingScope::all_accounts(),
        )),
        true,
        Default::default(),
    );
    let snapshot = gateway_core::runtime::RuntimeSnapshotHandle::new(
        gateway_core::routing::RuntimeSnapshot::new(
            gateway_core::routing::ConfigRevision::new(1).unwrap(),
            gateway_core::settings::SettingsValues::new(1, "smart", Default::default(), None, None),
            vec![provider],
            vec![],
            vec![policy],
        )
        .unwrap(),
    );
    let task = EvaluationTask::new(
        repository.clone(),
        execution.clone(),
        Arc::new(admin_observability_store(&db.pool)),
        "test".to_owned(),
        Default::default(),
        snapshot.clone(),
    );
    let service = EvaluationService::new(
        repository.clone(),
        "test".to_owned(),
        Default::default(),
        snapshot,
    );
    (repository, execution, task, service)
}
fn start(
    task: EvaluationTask,
) -> (
    CancellationToken,
    tokio::task::JoinHandle<Result<(), gateway_core::task::WorkerTaskError>>,
) {
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    (
        cancellation,
        tokio::spawn(async move { task.run(WorkerDaemonContext::new(token)).await }),
    )
}
async fn wait_finished(repository: &PgEvaluationRepository, id: &str) -> EvaluationRunDetail {
    // 模拟执行器不写用量，四项任务均需经过有界的观测等待与调度周期
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let detail = repository.load_run(id).await.unwrap().unwrap();
            if detail.run.active_items == 0 {
                return detail;
            }
            // 降低完整详情的读取频率，减少与被测 Worker 的数据库争用
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("任务在期限内完成")
}
async fn wait_started(execution: &Execution) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while execution.accounts.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn idle_worker_reports_health_after_successful_store_maintenance() {
    let Some(db) = TestDatabase::create("eval_worker_health").await else {
        return;
    };
    let (_, execution, task, _) = setup(&db, Reply::Answer).await;
    let daemon = WorkerDaemonContext::new(CancellationToken::new());
    let worker_context = daemon.clone();
    let handle = tokio::spawn(async move { task.run(worker_context).await });
    tokio::time::timeout(Duration::from_secs(10), daemon.progress_since(0))
        .await
        .expect("空队列完成仓储检查后应报告健康");
    daemon.cancellation().cancel();
    handle.await.unwrap().unwrap();
    assert!(execution.accounts.lock().unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn worker_does_not_report_health_when_store_recovery_fails() {
    let Some(db) = TestDatabase::create("eval_worker_health_failure").await else {
        return;
    };
    let (_, _, task, _) = setup(&db, Reply::Answer).await;
    let daemon = WorkerDaemonContext::new(CancellationToken::new());
    db.pool.close().await;
    assert!(task.run(daemon.clone()).await.is_err());
    assert_eq!(daemon.completed_cycles(), 0);
    db.close().await;
}

#[tokio::test]
async fn worker_persists_answers_before_commit_and_keeps_wrong_answers_as_success() {
    let Some(db) = TestDatabase::create("eval_worker_answer").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Answer).await;
    let run = service
        .submit(
            uuid::Uuid::new_v4(),
            parameters(&["acct_right", "acct_wrong"], 2),
            &context(),
        )
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.run.succeeded_items, 4);
    assert_eq!(detail.run.correct_items, 2);
    assert_eq!(execution.commits.load(Ordering::SeqCst), 4);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 4);
    assert!(
        detail
            .items
            .iter()
            .all(|item| item.request_id.is_some() && item.has_output)
    );
    assert!(
        detail
            .items
            .iter()
            .all(|item| !item.observation.request_history_available && item.ttft_ms.is_none())
    );
    assert_eq!(
        detail
            .items
            .iter()
            .filter(|item| item.verdict == EvaluationVerdict::Wrong)
            .count(),
        2
    );
    db.close().await;
}

#[tokio::test]
async fn stop_cancels_active_request_preserves_partial_text_and_skips_pending_items() {
    let Some(db) = TestDatabase::create("eval_worker_stop").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Pending).await;
    let run = service
        .submit(
            uuid::Uuid::new_v4(),
            parameters(&["acct_pending"], 3),
            &context(),
        )
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    wait_started(&execution).await;
    service.stop(&run.id, &context()).await.unwrap();
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(execution.accounts.lock().unwrap().len(), 1);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(execution.commits.load(Ordering::SeqCst), 0);
    let item = detail
        .items
        .iter()
        .find(|item| item.status == EvaluationItemStatus::Cancelled)
        .unwrap();
    assert_eq!(item.verdict, EvaluationVerdict::Ungraded);
    assert_eq!(
        repository
            .load_output(&item.id)
            .await
            .unwrap()
            .unwrap()
            .response_text,
        "部分回答"
    );
    assert_eq!(
        detail
            .items
            .iter()
            .filter(|item| item.status == EvaluationItemStatus::Skipped)
            .count(),
        2
    );
    db.close().await;
}

#[tokio::test]
async fn shutdown_drains_active_request_and_restart_does_not_replay_claimed_work() {
    let Some(db) = TestDatabase::create("eval_worker_restart").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Pending).await;
    let run = service
        .submit(
            uuid::Uuid::new_v4(),
            parameters(&["acct_pending"], 2),
            &context(),
        )
        .await
        .unwrap();
    let claimed = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    repository
        .record_request_id(&claimed.item.id, "req_before_crash")
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    wait_started(&execution).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let detail = repository.load_run(&run.id).await.unwrap().unwrap();
    assert_eq!(execution.accounts.lock().unwrap().len(), 1);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(
        detail
            .items
            .iter()
            .filter(|item| item.status == EvaluationItemStatus::Interrupted)
            .count(),
        1
    );
    assert_eq!(
        detail
            .items
            .iter()
            .filter(|item| item.status == EvaluationItemStatus::Cancelled)
            .count(),
        1
    );
    assert_eq!(detail.run.active_items, 0);
    db.close().await;
}

#[tokio::test]
async fn output_and_finalization_failures_never_report_a_successful_grade() {
    for reply in [Reply::Oversized, Reply::CommitFailure] {
        let Some(db) = TestDatabase::create("eval_worker_failure").await else {
            return;
        };
        let (repository, execution, task, service) = setup(&db, reply).await;
        let run = service
            .submit(
                uuid::Uuid::new_v4(),
                parameters(&["acct_right"], 1),
                &context(),
            )
            .await
            .unwrap();
        let (cancel, handle) = start(task);
        let detail = wait_finished(&repository, &run.id).await;
        cancel.cancel();
        handle.await.unwrap().unwrap();
        assert_eq!(detail.items[0].status, EvaluationItemStatus::Failed);
        assert_eq!(detail.items[0].verdict, EvaluationVerdict::Ungraded);
        assert_eq!(execution.commits.load(Ordering::SeqCst), 0);
        assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
        db.close().await;
    }
}

#[tokio::test]
async fn output_storage_failure_finalizes_core_before_worker_returns_error() {
    let Some(db) = TestDatabase::create("eval_worker_store_failure").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Answer).await;
    sqlx::query("alter table evaluation_artifacts add constraint test_reject_output check (false)")
        .execute(&db.pool)
        .await
        .unwrap();
    let run = service
        .submit(
            uuid::Uuid::new_v4(),
            parameters(&["acct_right"], 1),
            &context(),
        )
        .await
        .unwrap();
    let (_cancel, handle) = start(task);
    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert_eq!(execution.commits.load(Ordering::SeqCst), 0);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
    assert_eq!(repository.recover_running(Utc::now()).await.unwrap(), 1);
    assert_eq!(
        repository.load_run(&run.id).await.unwrap().unwrap().items[0].status,
        EvaluationItemStatus::Interrupted
    );
    db.close().await;
}

#[tokio::test]
async fn scheduled_work_skips_missed_slots_and_uses_the_saved_experiment() {
    let Some(db) = TestDatabase::create("eval_worker_schedule").await else {
        return;
    };
    let (repository, _execution, task, service) = setup(&db, Reply::Answer).await;
    let now = Utc::now();
    let plan = repository
        .save_plan(
            SaveEvaluationPlan {
                id: "eval_plan_due".to_owned(),
                expected_revision: None,
                settings: EvaluationPlanSettings {
                    account_preference: None,
                    account_selection: Default::default(),
                    name: "定时检查".to_owned(),
                    parameters: parameters(&["acct_right"], 1),
                    enabled: true,
                    interval_minutes: None,
                    cron_expression: "* * * * *".to_owned(),
                    timezone: Default::default(),
                },
                next_run_at: Some(now - chrono::Duration::hours(3)),
            },
            &context(),
        )
        .await
        .unwrap();
    task.advance_schedules(now).await.unwrap();
    task.advance_schedules(now).await.unwrap();
    let runs = service.list_runs(100, 0).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].plan_id.as_deref(), Some(plan.id.as_str()));
    assert_eq!(runs[0].active_items, 0);
    assert_eq!(runs[0].parameters, plan.settings.parameters);
    assert_eq!(
        service.detail(&runs[0].id).await.unwrap().items[0].status,
        EvaluationItemStatus::Skipped
    );
    let future = repository.list_plans().await.unwrap().remove(0);
    task.advance_schedules(future.next_run_at.unwrap())
        .await
        .unwrap();
    let runs = service.list_runs(100, 0).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs.iter().filter(|run| run.active_items == 1).count(), 1);
    db.close().await;
}

#[tokio::test]
async fn deployment_timezone_change_rebases_the_next_slot_without_replaying_the_old_one() {
    let Some(db) = TestDatabase::create("eval_worker_timezone").await else {
        return;
    };
    let (repository, _execution, task, service) = setup(&db, Reply::Answer).await;
    let now = Utc::now();
    let plan = repository
        .save_plan(
            SaveEvaluationPlan {
                id: "eval_plan_timezone".to_owned(),
                expected_revision: None,
                settings: EvaluationPlanSettings {
                    account_preference: None,
                    account_selection: Default::default(),
                    name: "每天检查".to_owned(),
                    parameters: parameters(&["acct_right"], 1),
                    enabled: true,
                    interval_minutes: None,
                    cron_expression: "0 9 * * *".to_owned(),
                    timezone: "America/New_York".parse().unwrap(),
                },
                next_run_at: Some(now - chrono::Duration::minutes(1)),
            },
            &context(),
        )
        .await
        .unwrap();
    task.advance_schedules(now).await.unwrap();
    let rebased = repository.list_plans().await.unwrap().remove(0);
    assert_eq!(rebased.settings.timezone.name(), "Asia/Shanghai");
    assert_eq!(rebased.revision, plan.revision + 1);
    assert!(rebased.next_run_at.unwrap() > now);
    assert!(service.list_runs(100, 0).await.unwrap().is_empty());
    task.advance_schedules(rebased.next_run_at.unwrap())
        .await
        .unwrap();
    assert_eq!(service.list_runs(100, 0).await.unwrap()[0].active_items, 1);
    db.close().await;
}

#[tokio::test]
async fn minute_plan_advances_atomically_without_drift_or_timezone_rebase() {
    let Some(db) = TestDatabase::create("eval_worker_interval").await else {
        return;
    };
    let (repository, _execution, task, service) = setup(&db, Reply::Answer).await;
    let now = Utc::now();
    let plan = service
        .save_plan(
            None,
            None,
            EvaluationPlanSettings {
                account_preference: None,
                account_selection: Default::default(),
                name: "45 分钟检查".to_owned(),
                parameters: parameters(&["acct_right"], 1),
                enabled: true,
                interval_minutes: Some(45),
                cron_expression: String::new(),
                timezone: Default::default(),
            },
            &context(),
        )
        .await
        .unwrap();
    let first = plan.next_run_at.unwrap();
    assert!(first >= now + chrono::Duration::minutes(45));
    // 模拟部署时区变更，分钟间隔仍使用同一 UTC 游标
    let mut settings = plan.settings;
    settings.timezone = "America/New_York".parse().unwrap();
    let plan = repository
        .save_plan(
            SaveEvaluationPlan {
                id: plan.id,
                expected_revision: Some(plan.revision),
                settings,
                next_run_at: Some(first),
            },
            &context(),
        )
        .await
        .unwrap();
    task.advance_schedules(first + chrono::Duration::seconds(29))
        .await
        .unwrap();
    task.advance_schedules(first + chrono::Duration::seconds(29))
        .await
        .unwrap();
    let runs = service.list_runs(100, 0).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].active_items, 1);
    assert_eq!(runs[0].scheduled_at, Some(first));
    let future = repository.list_plans().await.unwrap().remove(0);
    assert_eq!(future.revision, plan.revision);
    assert_eq!(
        future.next_run_at,
        Some(first + chrono::Duration::minutes(45))
    );
    // 停机错过多个时段时只记录一次跳过，并保持原来的间隔相位
    task.advance_schedules(first + chrono::Duration::minutes(100))
        .await
        .unwrap();
    let runs = service.list_runs(100, 0).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs.iter().filter(|run| run.active_items > 0).count(), 1);
    let future = repository.list_plans().await.unwrap().remove(0);
    assert_eq!(
        future.next_run_at,
        Some(first + chrono::Duration::minutes(135))
    );
    let mut paused = future.settings;
    paused.enabled = false;
    let paused = service
        .save_plan(Some(future.id), Some(future.revision), paused, &context())
        .await
        .unwrap();
    assert_eq!(paused.next_run_at, None);
    let mut resumed = paused.settings;
    resumed.enabled = true;
    resumed.interval_minutes = Some(90);
    let before_resume = Utc::now();
    let resumed = service
        .save_plan(Some(paused.id), Some(paused.revision), resumed, &context())
        .await
        .unwrap();
    assert!(resumed.next_run_at.unwrap() >= before_resume + chrono::Duration::minutes(90));
    assert_eq!(resumed.settings.interval_minutes, Some(90));
    db.close().await;
}

fn model_parameters() -> EvaluationParameters {
    let mut parameters = parameters(&["acct_right"], 1);
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.grader = EvaluationGrader::Model {
        config: EvaluationJudgeConfig {
            client_key_id: "key_evaluation".to_owned(),
            account_id: "acct_judge".to_owned(),
            model: PublicModelId::new("test-model").unwrap(),
            reasoning_effort: Some("high".to_owned()),
            timeout_seconds: 60,
            rubric: "检查最终答案与推导是否一致".to_owned(),
            reference_answer: None,
        },
    };
    parameters.definition = Some(definition);
    parameters
}

#[tokio::test]
async fn judge_commits_saved_output_and_invalid_json_never_becomes_a_wrong_answer() {
    for (reply, status, verdict) in [
        (
            Reply::Answer,
            EvaluationItemStatus::Succeeded,
            EvaluationVerdict::Wrong,
        ),
        (
            Reply::JudgeInvalid,
            EvaluationItemStatus::Failed,
            EvaluationVerdict::Ungraded,
        ),
    ] {
        let Some(db) = TestDatabase::create("eval_worker_judge").await else {
            return;
        };
        let (repository, execution, task, service) = setup(&db, reply).await;
        let run = service
            .submit(uuid::Uuid::new_v4(), model_parameters(), &context())
            .await
            .unwrap();
        let (cancel, handle) = start(task);
        let detail = wait_finished(&repository, &run.id).await;
        cancel.cancel();
        handle.await.unwrap().unwrap();
        assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
        assert_eq!(detail.items[0].verdict, verdict);
        assert_eq!(detail.items[0].judgment.as_ref().unwrap().status, status);
        assert_eq!(
            *execution.accounts.lock().unwrap(),
            ["acct_right", "acct_judge"]
        );
        assert_eq!(execution.commits.load(Ordering::SeqCst), 2);
        assert_eq!(execution.cleanups.load(Ordering::SeqCst), 2);
        db.close().await;
    }
}

#[tokio::test]
async fn stopping_a_judge_preserves_the_answer_and_finalizes_the_judge_session() {
    let Some(db) = TestDatabase::create("eval_worker_stop_judge").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::JudgePending).await;
    let run = service
        .submit(uuid::Uuid::new_v4(), model_parameters(), &context())
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    tokio::time::timeout(Duration::from_secs(15), async {
        while execution.accounts.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    service.stop(&run.id, &context()).await.unwrap();
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
    assert_eq!(detail.items[0].verdict, EvaluationVerdict::Ungraded);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Cancelled
    );
    assert!(
        repository
            .load_output(&detail.items[0].id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(execution.commits.load(Ordering::SeqCst), 1);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 2);
    db.close().await;
}
