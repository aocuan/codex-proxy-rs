//! 独立判题的持久状态、并发占用、重试与停止恢复

use chrono::Utc;
use gateway_admin::{model::evaluation::*, ports::evaluation::EvaluationRepository};
use gateway_core::routing::PublicModelId;
use gateway_store::postgres::PgEvaluationRepository;

use super::{
    super::TestDatabase,
    transactions::{completion, context, parameters},
};

async fn generated(repository: &PgEvaluationRepository) -> (String, String) {
    let mut parameters = parameters(&["acct_target"], 1);
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.grader = EvaluationGrader::Model {
        config: EvaluationJudgeConfig {
            client_key_id: "key_judge".to_owned(),
            account_id: "acct_judge".to_owned(),
            model: PublicModelId::new("judge-model").unwrap(),
            reasoning_effort: None,
            timeout_seconds: 30,
            rubric: "答案为 21".to_owned(),
            reference_answer: Some("21".to_owned()),
        },
    };
    parameters.definition = Some(definition);
    let run = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(parameters, "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    let item = repository
        .claim_next(Utc::now())
        .await
        .unwrap()
        .unwrap()
        .item;
    repository
        .save_output(
            &item.id,
            EvaluationOutput {
                response_text: "最终答案：21".to_owned(),
                artifact_html: None,
            },
        )
        .await
        .unwrap();
    let mut result = completion(EvaluationItemStatus::Succeeded);
    result.judge_target = Some(EvaluationJudgeTarget::Account("acct_judge".to_owned()));
    repository.finish_item(&item.id, result).await.unwrap();
    (run.id, item.id)
}

fn judged(
    status: EvaluationItemStatus,
    verdict: EvaluationVerdict,
) -> EvaluationJudgmentCompletion {
    EvaluationJudgmentCompletion {
        status,
        verdict,
        reason: "评审理由".to_owned(),
        observation: Default::default(),
        completed_at: Utc::now(),
    }
}

#[tokio::test]
async fn independent_judgment_preserves_generation_and_projects_the_latest_verdict() {
    let Some(db) = TestDatabase::create("evaluation_judgment").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let (run, item) = generated(&repository).await;
    let detail = repository.load_run(&run).await.unwrap().unwrap();
    assert_eq!(detail.run.succeeded_items, 1);
    assert_eq!(detail.run.active_items, 1);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Queued
    );
    let claim = repository
        .claim_judgment(Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.response_text, "最终答案：21");
    repository
        .record_judgment_request(&claim.judgment.id, "req_judge")
        .await
        .unwrap();
    assert!(
        !repository
            .finish_judgment(
                &claim.judgment.id,
                judged(EvaluationItemStatus::Succeeded, EvaluationVerdict::Wrong)
            )
            .await
            .unwrap()
    );
    repository
        .save_judgment_output(&claim.judgment.id, r#"{"pass":false,"reason":"推导错误"}"#)
        .await
        .unwrap();
    assert!(
        repository
            .finish_judgment(
                &claim.judgment.id,
                judged(EvaluationItemStatus::Succeeded, EvaluationVerdict::Wrong)
            )
            .await
            .unwrap()
    );
    let detail = repository.load_run(&run).await.unwrap().unwrap();
    assert_eq!(detail.run.active_items, 0);
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
    assert_eq!(detail.items[0].verdict, EvaluationVerdict::Wrong);
    assert!(repository.retry_judgment(&item, &context()).await.is_err());
    db.close().await;
}

#[tokio::test]
async fn failed_judgment_retries_without_regeneration_and_stopping_cancels_the_retry() {
    let Some(db) = TestDatabase::create("evaluation_judgment_retry").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let (run, item) = generated(&repository).await;
    let claim = repository
        .claim_judgment(Utc::now())
        .await
        .unwrap()
        .unwrap();
    repository
        .finish_judgment(
            &claim.judgment.id,
            judged(EvaluationItemStatus::Failed, EvaluationVerdict::Ungraded),
        )
        .await
        .unwrap();
    repository.retry_judgment(&item, &context()).await.unwrap();
    assert!(repository.retry_judgment(&item, &context()).await.is_err());
    let detail = repository.load_run(&run).await.unwrap().unwrap();
    assert_eq!(detail.items[0].judgment.as_ref().unwrap().attempt, 2);
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
    assert!(repository.claim_next(Utc::now()).await.unwrap().is_none());
    assert!(repository.stop_run(&run, &context()).await.unwrap());
    assert!(
        repository
            .claim_judgment(Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    let detail = repository.load_run(&run).await.unwrap().unwrap();
    assert_eq!(detail.run.active_items, 0);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Cancelled
    );
    assert_eq!(
        repository
            .load_output(&item)
            .await
            .unwrap()
            .unwrap()
            .response_text,
        "最终答案：21"
    );
    let attempts: i64 =
        sqlx::query_scalar("select count(*) from evaluation_judgments where item_id = $1")
            .bind(item)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(attempts, 2);
    db.close().await;
}

#[tokio::test]
async fn judge_accounts_share_the_execution_limit_and_recovery_does_not_replay_calls() {
    let Some(db) = TestDatabase::create("evaluation_judgment_recovery").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let (run, _) = generated(&repository).await;
    repository
        .claim_judgment(Utc::now())
        .await
        .unwrap()
        .unwrap();
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(parameters(&["acct_judge"], 1), "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    assert!(repository.claim_next(Utc::now()).await.unwrap().is_none());
    assert_eq!(repository.recover_running(Utc::now()).await.unwrap(), 1);
    assert!(
        repository
            .claim_judgment(Utc::now())
            .await
            .unwrap()
            .is_none()
    );
    let detail = repository.load_run(&run).await.unwrap().unwrap();
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Interrupted
    );
    assert!(repository.claim_next(Utc::now()).await.unwrap().is_some());
    db.close().await;
}
