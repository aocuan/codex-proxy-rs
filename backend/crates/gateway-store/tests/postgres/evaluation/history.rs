//! 账号状态来源的自动判定资格、条件隔离与测票事实读取

use chrono::Utc;
use gateway_admin::{model::evaluation::*, ports::evaluation::EvaluationRepository};
use gateway_core::routing::PublicModelId;
use gateway_store::postgres::PgEvaluationRepository;

use super::{
    super::TestDatabase,
    transactions::{completion, context, parameters},
};

fn seed(parameters: EvaluationParameters) -> EvaluationRunSeed {
    EvaluationRunSeed::new(parameters, "test-version", Utc::now())
}

#[tokio::test]
async fn sources_exclude_ungraded_artifacts_and_bind_history_to_one_experiment() {
    let Some(db) = TestDatabase::create("evaluation_history_sources").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            seed(parameters(&["acct_a", "acct_b"], 2)),
            &context(),
        )
        .await
        .unwrap();
    let original = repository.history_sources().await.unwrap().remove(0);
    let mut custom = parameters(&["acct_a"], 3);
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.name = "自定义题目".to_owned();
    definition.prompt = "计算 6 × 7".to_owned();
    definition.grader = EvaluationGrader::Exact {
        source: EvaluationAnswerSource::FullText,
        expected: "42".to_owned(),
    };
    custom.definition = Some(definition);
    let selected_run = repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(custom.clone()), &context())
        .await
        .unwrap();
    let selected = repository.history_sources().await.unwrap().remove(0);
    let mut different_model = custom.clone();
    different_model.model = PublicModelId::new("other-model").unwrap();
    let mut different_effort = custom.clone();
    different_effort.reasoning_effort = None;
    let mut different_grader = seed(custom.clone());
    different_grader.profile.grader_version = "other-grader".to_owned();
    for variant in [
        seed(different_model),
        seed(different_effort),
        different_grader,
    ] {
        repository
            .enqueue_manual(uuid::Uuid::new_v4(), variant, &context())
            .await
            .unwrap();
    }
    let mut artifact = parameters(&["acct_a"], 1);
    artifact.definition = Some(EvaluationDefinition::preset(EvaluationKind::Pelican));
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(artifact), &context())
        .await
        .unwrap();
    let sources = repository.history_sources().await.unwrap();
    assert_eq!(sources.len(), 5);
    assert!(sources.iter().all(|source| source.grader_type != "none"));
    let accounts = ["acct_a".to_owned(), "acct_b".to_owned()];
    assert!(
        repository
            .account_history(&accounts, 2, None)
            .await
            .unwrap()
            .is_empty()
    );
    let history = repository
        .account_history(&accounts, 2, Some(&selected.id))
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .all(|item| item.account_id == "acct_a" && item.run_id == selected_run.id)
    );
    let history = repository
        .account_history(&accounts, 2, Some(&original.id))
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|item| item.account_id == "acct_a")
            .count(),
        2
    );
    assert_eq!(
        history
            .iter()
            .filter(|item| item.account_id == "acct_b")
            .count(),
        2
    );
    custom.definition.as_mut().unwrap().name = "仅修改名称".to_owned();
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(custom), &context())
        .await
        .unwrap();
    let renamed = repository.history_sources().await.unwrap().remove(0);
    assert_eq!(renamed.id, selected.id);
    assert_eq!(renamed.name, "仅修改名称");
    assert!(
        repository
            .account_history(&accounts, 2, Some(&"0".repeat(32)))
            .await
            .unwrap()
            .is_empty()
    );
    db.close().await;
}

#[tokio::test]
async fn default_history_uses_quick_probe_conditions_and_keeps_outcomes_separate_from_verdicts() {
    let Some(db) = TestDatabase::create("evaluation_history_turnstate").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let mut probe = parameters(&["acct_a", "acct_b"], 1);
    probe.kind = EvaluationKind::Turnstate;
    probe.reasoning_effort = Some("low".to_owned());
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(probe.clone()), &context())
        .await
        .unwrap();
    for _ in 0..2 {
        let claimed = repository.claim_next(Utc::now()).await.unwrap().unwrap();
        let outcome = if claimed.item.account_id == "acct_a" {
            TurnStateOutcome::Accepted
        } else {
            TurnStateOutcome::Unknown
        };
        assert!(
            repository
                .save_output(
                    &claimed.item.id,
                    EvaluationOutput {
                        response_text: "测票诊断".to_owned(),
                        artifact_html: None,
                    }
                )
                .await
                .unwrap()
        );
        let mut finished = completion(EvaluationItemStatus::Succeeded);
        finished.observation.turn_state = Some(TurnStateProbeResult {
            outcome,
            steps: vec![],
            error: None,
        });
        assert!(
            repository
                .finish_item(&claimed.item.id, finished)
                .await
                .unwrap()
        );
    }
    let source = repository.history_sources().await.unwrap().remove(0);
    assert_eq!(source.kind, EvaluationKind::Turnstate);
    assert_eq!(source.name, "快速测票");
    assert_eq!(source.grader_type, "none");
    let accounts = ["acct_a".to_owned(), "acct_b".to_owned()];
    let history = repository
        .account_history(&accounts, 28, None)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].turn_state, Some(TurnStateOutcome::Accepted));
    assert_eq!(history[1].turn_state, Some(TurnStateOutcome::Unknown));
    assert!(
        history.iter().all(
            |item| item.verdict == EvaluationVerdict::Ungraded && item.duration_ms == Some(100)
        )
    );
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(probe.clone()), &context())
        .await
        .unwrap();
    let pending = repository
        .account_history(&accounts, 28, Some(&source.id))
        .await
        .unwrap();
    assert_eq!(pending.len(), 4);
    assert_eq!(pending[0].status, EvaluationItemStatus::Queued);
    assert_eq!(pending[1].turn_state, Some(TurnStateOutcome::Accepted));
    probe.model = PublicModelId::new("other-model").unwrap();
    probe.account_ids = vec!["acct_a".to_owned()];
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed(probe), &context())
        .await
        .unwrap();
    let latest = repository
        .account_history(&accounts, 28, None)
        .await
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].model.as_str(), "other-model");
    assert_eq!(
        repository
            .account_history(&accounts, 28, Some(&source.id))
            .await
            .unwrap()
            .len(),
        4
    );
    db.close().await;
}
