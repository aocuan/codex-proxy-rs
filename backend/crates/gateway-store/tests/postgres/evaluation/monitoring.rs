//! 正式监测的来源隔离、复测去重、方案版本和到期回收事务

use chrono::Utc;
use gateway_admin::{
    model::{Revision, evaluation::*},
    ports::evaluation::EvaluationRepository,
};
use gateway_store::postgres::PgEvaluationRepository;

use super::{
    super::TestDatabase,
    transactions::{completion, context, parameters},
};

fn probe(ids: &[&str]) -> EvaluationParameters {
    let mut value = parameters(ids, 1);
    value.kind = EvaluationKind::Turnstate;
    value.reasoning_effort = Some("low".into());
    value
}

fn settings() -> EvaluationPlanSettings {
    EvaluationPlanSettings {
        monitoring: Some(Default::default()),
        account_selection: EvaluationAccountSelection::All,
        account_preference: None,
        name: "正式方案".into(),
        parameters: probe(&[]),
        enabled: true,
        interval_minutes: Some(10),
        cron_expression: String::new(),
        timezone: Default::default(),
    }
}

async fn accounts(db: &TestDatabase) {
    sqlx::query("insert into provider_accounts (id,provider_kind,name,authentication_kind,provider_credentials_json,has_refresh_token,credential_observed_at,created_at,updated_at) select id,'openai',id,'oauth','{}'::jsonb,false,now(),now(),now() from unnest($1::text[]) id")
        .bind(vec!["acct_a", "acct_b"]).execute(&db.pool).await.unwrap();
}

#[tokio::test]
async fn manual_diagnostics_do_not_select_a_source_and_formal_rechecks_reuse_pending_work() {
    let Some(db) = TestDatabase::create("monitoring_source_and_recheck").await else {
        return;
    };
    accounts(&db).await;
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let formal = repository
        .save_plan(
            SaveEvaluationPlan {
                id: "eval_plan_monitor".into(),
                expected_revision: None,
                settings: settings(),
                next_run_at: Some(Utc::now()),
            },
            &context(),
        )
        .await
        .unwrap();
    let mut seed = EvaluationRunSeed::new(probe(&["acct_a", "acct_b"]), "test", Utc::now());
    seed.profile.monitoring = Some(formal.monitoring_reference());
    let run = repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed.clone(), &context())
        .await
        .unwrap();
    let mut subset = seed.clone();
    subset.parameters.account_ids = vec!["acct_a".into()];
    assert_eq!(
        repository
            .enqueue_manual(uuid::Uuid::new_v4(), subset, &context())
            .await
            .unwrap()
            .id,
        run.id
    );
    assert_eq!(repository.list_runs(100, 0).await.unwrap().len(), 1);

    let mut diagnostic = probe(&["acct_a"]);
    diagnostic.model = gateway_core::routing::PublicModelId::new("other-model").unwrap();
    let diagnostic = repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(diagnostic, "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    let ids = vec!["acct_a".into(), "acct_b".into()];
    let history = repository.account_history(&ids, 28, None).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|point| point.run_id == run.id));
    let source = repository
        .history_sources()
        .await
        .unwrap()
        .into_iter()
        .find(|source| source.model.as_str() == "other-model")
        .unwrap();
    assert_eq!(
        repository
            .account_history(&ids, 28, Some(&source.id))
            .await
            .unwrap()[0]
            .run_id,
        diagnostic.id
    );

    let renamed = repository
        .save_plan(
            SaveEvaluationPlan {
                id: formal.id.clone(),
                expected_revision: Some(formal.revision),
                settings: EvaluationPlanSettings {
                    name: "仅改名称".into(),
                    ..formal.settings.clone()
                },
                next_run_at: formal.next_run_at,
            },
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(
        renamed.monitoring_reference().version,
        formal.monitoring_reference().version
    );
    assert_eq!(
        repository
            .account_history(&ids, 28, None)
            .await
            .unwrap()
            .len(),
        2
    );
    let mut changed = renamed.settings.clone();
    changed.parameters.model =
        gateway_core::routing::PublicModelId::new("new-monitor-model").unwrap();
    let changed = repository
        .save_plan(
            SaveEvaluationPlan {
                id: renamed.id,
                expected_revision: Some(renamed.revision),
                settings: changed,
                next_run_at: renamed.next_run_at,
            },
            &context(),
        )
        .await
        .unwrap();
    assert_ne!(
        changed.monitoring_reference().version,
        formal.monitoring_reference().version
    );
    assert!(
        repository
            .account_history(&ids, 28, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repository
            .enqueue_manual(uuid::Uuid::new_v4(), seed, &context())
            .await
            .is_err()
    );
    assert_eq!(
        repository
            .load_run(&run.id)
            .await
            .unwrap()
            .unwrap()
            .items
            .len(),
        2
    );
    db.close().await;
}

#[tokio::test]
async fn expired_membership_is_audited_once_and_new_results_fence_stale_cleanup() {
    let Some(db) = TestDatabase::create("monitoring_expiration").await else {
        return;
    };
    accounts(&db).await;
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let group = format!("grp_{}", "a".repeat(32));
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'preferred','#FFFFFFFF',now(),now())")
        .bind(&group).execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_a',now()),($1,'acct_b',now())")
        .bind(&group).execute(&db.pool).await.unwrap();
    let formal = repository
        .save_plan(
            SaveEvaluationPlan {
                id: "eval_plan_expire".into(),
                expected_revision: None,
                settings: EvaluationPlanSettings {
                    monitoring: Some(EvaluationMonitoringPolicy {
                        remove_expired: true,
                        ..Default::default()
                    }),
                    account_preference: Some(EvaluationAccountPreference {
                        group_id: group.clone(),
                    }),
                    ..settings()
                },
                next_run_at: Some(Utc::now()),
            },
            &context(),
        )
        .await
        .unwrap();
    let observed_at = Utc::now();
    let mut seed = EvaluationRunSeed::new(probe(&["acct_b"]), "test", Utc::now());
    seed.profile.monitoring = Some(formal.monitoring_reference());
    repository
        .enqueue_manual(uuid::Uuid::new_v4(), seed, &context())
        .await
        .unwrap();
    let claim = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    repository
        .save_output(
            &claim.item.id,
            EvaluationOutput {
                response_text: "new observation".into(),
                artifact_html: None,
            },
        )
        .await
        .unwrap();
    let mut finished = completion(EvaluationItemStatus::Succeeded);
    finished.completed_at = observed_at - chrono::Duration::seconds(1);
    finished.observation.turn_state = Some(TurnStateProbeResult {
        diagnostic: None,
        connection_revision: None,
        outcome: TurnStateOutcome::Accepted,
        steps: vec![],
        error: None,
    });
    repository
        .finish_item(&claim.item.id, finished)
        .await
        .unwrap();
    let current: i64 =
        sqlx::query_scalar("select config_revision from runtime_settings where id=1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let command = EvaluationPreferenceCleanup {
        plan: formal.monitoring_reference(),
        accounts: ["acct_a", "acct_b"]
            .into_iter()
            .map(|id| EvaluationPreferenceCleanupTarget {
                account_id: id.into(),
                reason: EvaluationPreferenceCleanupReason::Expired {
                    last_signal_id: None,
                },
            })
            .collect(),
        expected_revision: Revision::new(current as u64).unwrap(),
    };
    assert!(
        repository
            .cleanup_preference(command.clone())
            .await
            .unwrap()
            .is_some()
    );
    let members: Vec<String> = sqlx::query_scalar("select provider_account_id from account_group_accounts where account_group_id=$1 order by provider_account_id")
        .bind(&group).fetch_all(&db.pool).await.unwrap();
    assert_eq!(members, ["acct_b"]);
    assert!(
        repository
            .cleanup_preference(command)
            .await
            .unwrap()
            .is_none()
    );
    let audits: i64 = sqlx::query_scalar(
        "select count(*) from admin_audit_events where admin_request_id like 'evaluation_expiration_%'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);
    db.close().await;
}
