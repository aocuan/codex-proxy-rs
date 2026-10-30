//! 测评事务在真实 PostgreSQL 中的幂等、并发及保守恢复合同

use chrono::{Duration, Utc};
use gateway_admin::{
    model::{MutationActor, MutationContext, evaluation::*},
    ports::{evaluation::EvaluationRepository, store::AdminStoreErrorKind},
};
use gateway_core::routing::PublicModelId;
use gateway_store::postgres::PgEvaluationRepository;

use super::super::TestDatabase;

pub(super) fn context() -> MutationContext {
    MutationContext {
        actor: MutationActor::System,
        request_id: "evaluation-test".to_owned(),
    }
}

pub(super) fn parameters(accounts: &[&str], repetitions: u8) -> EvaluationParameters {
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

fn seed(parameters: EvaluationParameters) -> EvaluationRunSeed {
    EvaluationRunSeed::new(parameters, "test-version", Utc::now())
}

pub(super) fn completion(status: EvaluationItemStatus) -> EvaluationCompletion {
    EvaluationCompletion {
        judge_target: None,
        account_preference: None,
        status,
        verdict: EvaluationVerdict::Ungraded,
        final_answer: None,
        error_code: None,
        message: None,
        ttft_ms: Some(50),
        duration_ms: 100,
        completed_at: Utc::now(),
        observation: Default::default(),
    }
}

async fn plan(repository: &PgEvaluationRepository) -> EvaluationPlan {
    repository
        .save_plan(
            SaveEvaluationPlan {
                id: format!("eval_plan_{}", uuid::Uuid::now_v7().simple()),
                expected_revision: None,
                settings: EvaluationPlanSettings {
                    account_preference: None,
                    account_selection: Default::default(),
                    name: "每小时测评".to_owned(),
                    parameters: parameters(&["acct_a"], 1),
                    enabled: true,
                    interval_minutes: None,
                    cron_expression: "0 * * * *".to_owned(),
                    timezone: Default::default(),
                },
                next_run_at: Some(Utc::now()),
            },
            &context(),
        )
        .await
        .unwrap()
}

fn schedule(plan: &EvaluationPlan) -> EnqueueScheduledEvaluation {
    let at = plan.next_run_at.unwrap();
    EnqueueScheduledEvaluation {
        plan_id: plan.id.clone(),
        expected_revision: plan.revision,
        scheduled_at: at,
        next_run_at: at + Duration::hours(1),
        seed: seed(plan.settings.parameters.clone()),
        skip_reason: None,
    }
}

#[tokio::test]
async fn manual_submission_is_scoped_idempotent_and_audited_once() {
    let Some(db) = TestDatabase::create("evaluation_idempotency").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let submission = uuid::Uuid::new_v4();
    let first = repository
        .enqueue_manual(submission, seed(parameters(&["acct_a"], 1)), &context())
        .await
        .unwrap();
    let replay = repository
        .enqueue_manual(submission, seed(parameters(&["acct_a"], 1)), &context())
        .await
        .unwrap();
    assert_eq!(first.id, replay.id);
    assert_eq!(replay.total_items, 1);
    assert_eq!(replay.active_items, 1);
    let error = repository
        .enqueue_manual(submission, seed(parameters(&["acct_b"], 1)), &context())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), AdminStoreErrorKind::Conflict);
    let audits: i64 = sqlx::query_scalar(
        "select count(*) from admin_audit_events where entity_kind = 'evaluation_run'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    let another_actor = MutationContext {
        actor: MutationActor::AdminApiKey,
        request_id: "other-admin".to_owned(),
    };
    let other = repository
        .enqueue_manual(submission, seed(parameters(&["acct_a"], 1)), &another_actor)
        .await
        .unwrap();
    assert_ne!(first.id, other.id);
    db.close().await;
}

#[tokio::test]
async fn concurrent_claims_respect_account_and_global_limits_and_stop_is_persistent() {
    let Some(db) = TestDatabase::create("evaluation_claim").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let run = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_a", "acct_b"], 2)),
            &context(),
        )
        .await
        .unwrap();
    let now = Utc::now();
    let (a, b, c) = tokio::join!(
        repository.claim_next(now),
        repository.claim_next(now),
        repository.claim_next(now)
    );
    let claimed: Vec<_> = [a.unwrap(), b.unwrap(), c.unwrap()]
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(claimed.len(), 2);
    assert_ne!(claimed[0].item.account_id, claimed[1].item.account_id);
    assert_eq!(claimed[0].profile.gateway_version, "test-version");
    assert!(repository.stop_run(&run.id, &context()).await.unwrap());
    assert!(!repository.stop_run(&run.id, &context()).await.unwrap());
    assert!(repository.is_stop_requested(&run.id).await.unwrap());
    let detail = repository.load_run(&run.id).await.unwrap().unwrap();
    assert_eq!(
        detail
            .items
            .iter()
            .filter(|item| item.status == EvaluationItemStatus::Skipped)
            .count(),
        2
    );
    assert_eq!(detail.run.active_items, 2);
    for claim in claimed {
        assert!(
            repository
                .finish_item(&claim.item.id, completion(EvaluationItemStatus::Cancelled))
                .await
                .unwrap()
                .finished
        );
    }
    assert!(repository.claim_next(now).await.unwrap().is_none());
    assert_eq!(
        repository
            .load_run(&run.id)
            .await
            .unwrap()
            .unwrap()
            .run
            .active_items,
        0
    );
    let audits: i64 = sqlx::query_scalar(
        "select count(*) from admin_audit_events where entity_kind = 'evaluation_run' and action = 'stop'",
    ).fetch_one(&db.pool).await.unwrap();
    assert_eq!(audits, 1);
    db.close().await;
}

#[tokio::test]
async fn restart_preserves_output_but_never_replays_or_grades_an_unconfirmed_request() {
    let Some(db) = TestDatabase::create("evaluation_restart").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let run = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_a"], 2)),
            &context(),
        )
        .await
        .unwrap();
    let claim = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    assert!(
        repository
            .record_request_id(&claim.item.id, "req_test")
            .await
            .unwrap()
    );
    assert!(
        !repository
            .record_request_id(&claim.item.id, "req_changed")
            .await
            .unwrap()
    );
    let output = EvaluationOutput {
        response_text: "最终答案：21".to_owned(),
        artifact_html: None,
    };
    assert!(
        repository
            .save_output(&claim.item.id, output.clone())
            .await
            .unwrap()
    );
    assert!(
        repository
            .save_output(&claim.item.id, output.clone())
            .await
            .unwrap()
    );
    let changed = EvaluationOutput {
        response_text: "最终答案：29".to_owned(),
        artifact_html: None,
    };
    assert_eq!(
        repository
            .save_output(&claim.item.id, changed)
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::Conflict
    );
    assert_eq!(repository.recover_running(Utc::now()).await.unwrap(), 1);
    assert_eq!(repository.recover_running(Utc::now()).await.unwrap(), 0);
    assert!(
        !repository
            .finish_item(&claim.item.id, completion(EvaluationItemStatus::Succeeded))
            .await
            .unwrap()
            .finished
    );
    assert_eq!(
        repository.load_output(&claim.item.id).await.unwrap(),
        Some(output)
    );
    let detail = repository.load_run(&run.id).await.unwrap().unwrap();
    let interrupted = detail
        .items
        .iter()
        .find(|item| item.id == claim.item.id)
        .unwrap();
    assert_eq!(interrupted.status, EvaluationItemStatus::Interrupted);
    assert_eq!(interrupted.verdict, EvaluationVerdict::Ungraded);
    assert_eq!(interrupted.request_id.as_deref(), Some("req_test"));
    assert!(interrupted.has_output);
    let next = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    assert_ne!(next.item.id, claim.item.id);
    assert_eq!(next.item.repetition, 2);
    db.close().await;
}

#[tokio::test]
async fn only_saved_output_can_succeed_and_wrong_answer_is_still_a_successful_execution() {
    let Some(db) = TestDatabase::create("evaluation_output").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let run = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_a"], 1)),
            &context(),
        )
        .await
        .unwrap();
    let claim = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    let mut result = completion(EvaluationItemStatus::Succeeded);
    result.verdict = EvaluationVerdict::Wrong;
    result.final_answer = Some(i64::from(u32::MAX));
    assert!(
        !repository
            .finish_item(&claim.item.id, result.clone())
            .await
            .unwrap()
            .finished
    );
    let oversized = EvaluationOutput {
        response_text: "x".repeat(MAX_RESPONSE_BYTES + 1),
        artifact_html: None,
    };
    assert_eq!(
        repository
            .save_output(&claim.item.id, oversized)
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::Invalid
    );
    repository
        .save_output(
            &claim.item.id,
            EvaluationOutput {
                response_text: format!("最终答案：{}", u32::MAX),
                artifact_html: None,
            },
        )
        .await
        .unwrap();
    assert!(
        repository
            .finish_item(&claim.item.id, result.clone())
            .await
            .unwrap()
            .finished
    );
    assert!(
        !repository
            .finish_item(&claim.item.id, result)
            .await
            .unwrap()
            .finished
    );
    let detail = repository.load_run(&run.id).await.unwrap().unwrap();
    assert_eq!(detail.run.succeeded_items, 1);
    assert_eq!(detail.run.correct_items, 0);
    assert_eq!(detail.items[0].verdict, EvaluationVerdict::Wrong);
    assert_eq!(detail.items[0].final_answer, Some(i64::from(u32::MAX)));
    db.close().await;
}

#[tokio::test]
async fn schedule_claim_is_atomic_and_overlap_is_recorded_without_another_call() {
    let Some(db) = TestDatabase::create("evaluation_schedule").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let plan = plan(&repository).await;
    let (a, b) = tokio::join!(
        repository.enqueue_scheduled(schedule(&plan)),
        repository.enqueue_scheduled(schedule(&plan))
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert_eq!(repository.list_runs(100, 0).await.unwrap().len(), 1);
    let advanced = repository.list_plans().await.unwrap().remove(0);
    assert_eq!(
        advanced.next_run_at,
        Some(plan.next_run_at.unwrap() + Duration::hours(1))
    );
    assert_eq!(advanced.revision, plan.revision);
    assert!(
        repository
            .enqueue_scheduled(schedule(&advanced))
            .await
            .unwrap()
    );
    let runs = repository.list_runs(100, 0).await.unwrap();
    assert_eq!(runs.len(), 2);
    let skipped = runs.iter().find(|run| run.active_items == 0).unwrap();
    let detail = repository.load_run(&skipped.id).await.unwrap().unwrap();
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Skipped);
    assert_eq!(
        detail.items[0].message.as_deref(),
        Some("上一轮测评尚未结束")
    );
    assert!(
        repository
            .delete_plan(&plan.id, plan.revision, &context())
            .await
            .unwrap()
    );
    assert_eq!(repository.list_runs(100, 0).await.unwrap().len(), 2);
    assert!(
        !repository
            .enqueue_scheduled(schedule(&advanced))
            .await
            .unwrap()
    );
    db.close().await;
}

#[tokio::test]
async fn pausing_a_plan_invalidates_stale_scheduler_and_editor_writes() {
    let Some(db) = TestDatabase::create("evaluation_plan_cas").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let original = plan(&repository).await;
    let mut settings = original.settings.clone();
    settings.enabled = false;
    let command = SaveEvaluationPlan {
        id: original.id.clone(),
        expected_revision: Some(original.revision),
        settings,
        next_run_at: None,
    };
    let paused = repository
        .save_plan(command.clone(), &context())
        .await
        .unwrap();
    assert_eq!(paused.revision, original.revision + 1);
    assert_eq!(
        repository
            .save_plan(command, &context())
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::StaleRevision
    );
    assert!(
        !repository
            .enqueue_scheduled(schedule(&original))
            .await
            .unwrap()
    );
    assert_eq!(
        repository
            .delete_plan(&original.id, original.revision, &context())
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::StaleRevision
    );
    assert!(repository.list_runs(100, 0).await.unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn full_queue_rejects_manual_submission_and_records_scheduled_skip() {
    let Some(db) = TestDatabase::create("evaluation_capacity").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let ids: Vec<_> = (0..20).map(|i| format!("acct_{i}")).collect();
    let mut full = parameters(&["acct_a"], 5);
    full.account_ids = ids;
    let run = repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(full), &context())
        .await
        .unwrap();
    assert_eq!(run.active_items, 100);
    let extra = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_extra"], 1)),
            &context(),
        )
        .await
        .unwrap_err();
    assert_eq!(extra.kind(), AdminStoreErrorKind::Conflict);
    let plan = plan(&repository).await;
    assert!(repository.enqueue_scheduled(schedule(&plan)).await.unwrap());
    let runs = repository.list_runs(100, 0).await.unwrap();
    let skipped = runs.iter().find(|run| run.plan_id.is_some()).unwrap();
    assert_eq!(skipped.active_items, 0);
    assert_eq!(
        repository
            .load_run(&skipped.id)
            .await
            .unwrap()
            .unwrap()
            .items[0]
            .message
            .as_deref(),
        Some("测评等待队列已满")
    );
    assert!(repository.stop_run(&run.id, &context()).await.unwrap());
    assert!(
        repository
            .enqueue_manual(
                uuid::Uuid::new_v4(),
                seed(parameters(&["acct_extra"], 1)),
                &context()
            )
            .await
            .is_ok()
    );
    db.close().await;
}

#[tokio::test]
async fn retention_is_bounded_and_keeps_active_items_and_original_batch_size() {
    let Some(db) = TestDatabase::create("evaluation_retention").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let run = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_a"], 5)),
            &context(),
        )
        .await
        .unwrap();
    for _ in 0..4 {
        let claim = repository.claim_next(Utc::now()).await.unwrap().unwrap();
        repository
            .save_output(
                &claim.item.id,
                EvaluationOutput {
                    response_text: "答案".repeat(100),
                    artifact_html: None,
                },
            )
            .await
            .unwrap();
        repository
            .finish_item(&claim.item.id, completion(EvaluationItemStatus::Succeeded))
            .await
            .unwrap();
    }
    let active = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    let mut policy = EvaluationRetention::standard(Utc::now());
    policy.keep_per_account_series = 2;
    policy.batch_size = 1;
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 1);
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 1);
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 0);
    let detail = repository.load_run(&run.id).await.unwrap().unwrap();
    assert_eq!(detail.run.total_items, 5);
    assert_eq!(detail.run.retained_items, 3);
    assert_eq!(detail.run.active_items, 1);
    assert!(detail.items.iter().any(|item| item.id == active.item.id));
    // 正文预算也能清理旧终态，但不能删除仍在运行的请求
    policy.max_output_bytes = 1;
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 1);
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 1);
    assert_eq!(repository.prune_history(policy.clone()).await.unwrap(), 0);
    repository
        .finish_item(&active.item.id, completion(EvaluationItemStatus::Cancelled))
        .await
        .unwrap();
    policy.completed_before = Utc::now() + Duration::days(1);
    assert_eq!(repository.prune_history(policy).await.unwrap(), 1);
    assert!(repository.load_run(&run.id).await.unwrap().is_none());
    db.close().await;
}

#[tokio::test]
async fn audit_failure_rolls_back_submission_and_all_child_items() {
    let Some(db) = TestDatabase::create("evaluation_audit_rollback").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    // 模拟审计存储拒绝写入，检查业务记录不能独立提交
    sqlx::query(
        "alter table admin_audit_events add constraint evaluation_test_reject
         check (entity_kind <> 'evaluation_run')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    assert!(
        repository
            .enqueue_manual(
                uuid::Uuid::new_v4(),
                seed(parameters(&["acct_a"], 1)),
                &context()
            )
            .await
            .is_err()
    );
    assert!(repository.list_runs(100, 0).await.unwrap().is_empty());
    assert!(repository.claim_next(Utc::now()).await.unwrap().is_none());
    db.close().await;
}
