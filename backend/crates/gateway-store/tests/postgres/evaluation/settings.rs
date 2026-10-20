//! 测评默认设置的并发保护、凭据保留与公开响应脱敏

use gateway_admin::{
    model::evaluation::*,
    ports::{evaluation::EvaluationRepository, store::AdminStoreErrorKind},
};
use gateway_store::postgres::PgEvaluationRepository;
use secrecy::{ExposeSecret as _, SecretString};

use super::{super::TestDatabase, transactions::context};

#[tokio::test]
async fn settings_protect_concurrent_edits_and_keep_credentials_out_of_public_data() {
    let Some(db) = TestDatabase::create("eval_settings").await else {
        return;
    };
    let repository = PgEvaluationRepository::new(db.pool.clone());
    let initial = repository.settings().await.unwrap();
    let defaults = EvaluationDefaults {
        judge: Some(EvaluationJudgeDefaults::Typesafe {
            model: "jev-1.13.0".to_owned(),
            decision_threshold: 80,
            timeout_seconds: 60,
        }),
        ..initial.defaults
    };
    assert!(
        repository
            .update_settings(
                UpdateEvaluationSettings {
                    revision: 0,
                    defaults: defaults.clone(),
                    typesafe_api_key: None,
                },
                &context()
            )
            .await
            .is_err()
    );
    let saved = repository
        .update_settings(
            UpdateEvaluationSettings {
                revision: 0,
                defaults: defaults.clone(),
                typesafe_api_key: Some(SecretString::from("PRIVATE_TEST_JUDGE_CREDENTIAL")),
            },
            &context(),
        )
        .await
        .unwrap();
    assert!(saved.typesafe_configured);
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("PRIVATE_TEST")
    );
    let stale = repository
        .update_settings(
            UpdateEvaluationSettings {
                revision: 0,
                defaults: defaults.clone(),
                typesafe_api_key: None,
            },
            &context(),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.kind(), AdminStoreErrorKind::StaleRevision);
    let retained = repository
        .update_settings(
            UpdateEvaluationSettings {
                revision: saved.revision,
                defaults,
                typesafe_api_key: None,
            },
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .typesafe_api_key()
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "PRIVATE_TEST_JUDGE_CREDENTIAL"
    );
    let cleared = repository
        .update_settings(
            UpdateEvaluationSettings {
                revision: retained.revision,
                defaults: EvaluationDefaults::default(),
                typesafe_api_key: Some(SecretString::from("")),
            },
            &context(),
        )
        .await
        .unwrap();
    assert!(!cleared.typesafe_configured);
    assert!(repository.typesafe_api_key().await.unwrap().is_none());
    db.close().await;
}
