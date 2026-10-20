//! 外部判题的结果投影、异常隔离和取消资源释放

use async_trait::async_trait;
use gateway_admin::ports::evaluation::TypeSafeJudgeClient;
use secrecy::{ExposeSecret as _, SecretString};

use super::*;

struct Judge {
    reply: Option<Result<String, &'static str>>,
    calls: AtomicUsize,
    released: AtomicUsize,
}

struct Release<'a>(&'a AtomicUsize);
impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl TypeSafeJudgeClient for Judge {
    async fn judge(
        &self,
        key: &SecretString,
        _: &TypeSafeJudgeConfig,
        definition: &EvaluationDefinition,
        response: &str,
    ) -> Result<String, &'static str> {
        assert_eq!(key.expose_secret(), "PRIVATE_TEST_JUDGE_CREDENTIAL");
        assert_eq!(definition.name, "糖果题");
        assert!(response.contains("最终答案：21"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _release = Release(&self.released);
        match &self.reply {
            Some(reply) => reply.clone(),
            None => std::future::pending().await,
        }
    }
}

async fn configure(repository: &PgEvaluationRepository) {
    repository
        .update_settings(
            UpdateEvaluationSettings {
                revision: 0,
                defaults: EvaluationDefaults::default(),
                typesafe_api_key: Some(SecretString::from("PRIVATE_TEST_JUDGE_CREDENTIAL")),
            },
            &context(),
        )
        .await
        .unwrap();
}

fn parameters() -> EvaluationParameters {
    let mut parameters = super::parameters(&["acct_target"], 1);
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.grader = EvaluationGrader::Typesafe {
        config: TypeSafeJudgeConfig {
            model: "jev-1.13.0".to_owned(),
            decision_threshold: 80,
            timeout_seconds: 30,
            rubric: "最终答案是 21".to_owned(),
            reference_answer: Some("21".to_owned()),
        },
    };
    parameters.definition = Some(definition);
    parameters
}

fn response(probability: f64) -> String {
    serde_json::json!({
        "model": "jev-1.13.0",
        "answers": { "meets_criteria": { "type": "noul", "noul": probability } },
        "usage": { "input_tokens": 123, "output_tokens": 24 },
    })
    .to_string()
}

#[tokio::test]
async fn external_judge_projects_verdicts_without_rebilling_generation_or_retrying_errors() {
    for (reply, verdict, succeeded) in [
        (Ok(response(0.99)), EvaluationVerdict::Correct, true),
        (Ok(response(0.01)), EvaluationVerdict::Wrong, true),
        (Ok(response(0.5)), EvaluationVerdict::Ungraded, true),
        (Ok(response(1.01)), EvaluationVerdict::Ungraded, false),
        (
            Ok("invalid JSON".to_owned()),
            EvaluationVerdict::Ungraded,
            false,
        ),
        (Err("TypeSafe 请求限流"), EvaluationVerdict::Ungraded, false),
    ] {
        let Some(db) = TestDatabase::create("eval_typesafe_result").await else {
            return;
        };
        let (repository, execution, task, service) = setup(&db, Reply::Answer);
        configure(&repository).await;
        let judge = Arc::new(Judge {
            reply: Some(reply),
            calls: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
        });
        let run = service
            .submit(uuid::Uuid::new_v4(), parameters(), &context())
            .await
            .unwrap();
        let (cancel, handle) = start(task.with_typesafe(Some(judge.clone())));
        let detail = wait_finished(&repository, &run.id).await;
        cancel.cancel();
        handle.await.unwrap().unwrap();
        let item = &detail.items[0];
        let judgment = item.judgment.as_ref().unwrap();
        assert_eq!(item.status, EvaluationItemStatus::Succeeded);
        assert_eq!(item.verdict, verdict);
        assert_eq!(
            judgment.status,
            if succeeded {
                EvaluationItemStatus::Succeeded
            } else {
                EvaluationItemStatus::Failed
            }
        );
        assert!(judgment.account_id.is_none());
        assert!(judgment.request_id.is_none());
        assert!(judgment.observation.cost_amount.is_none());
        assert!(!judgment.observation.request_history_available);
        assert_eq!(execution.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            execution.accounts.lock().unwrap().as_slice(),
            &["acct_target"]
        );
        assert_eq!(judge.calls.load(Ordering::SeqCst), 1);
        assert_eq!(judge.released.load(Ordering::SeqCst), 1);
        let history = repository
            .account_history(&["acct_target".to_owned()], 28)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].verdict, verdict);
        db.close().await;
    }
}

#[tokio::test]
async fn stop_drops_external_judge_future_without_losing_the_generated_answer() {
    let Some(db) = TestDatabase::create("eval_typesafe_cancel").await else {
        return;
    };
    let (repository, _, task, service) = setup(&db, Reply::Answer);
    configure(&repository).await;
    let judge = Arc::new(Judge {
        reply: None,
        calls: AtomicUsize::new(0),
        released: AtomicUsize::new(0),
    });
    let run = service
        .submit(uuid::Uuid::new_v4(), parameters(), &context())
        .await
        .unwrap();
    let (cancel, handle) = start(task.with_typesafe(Some(judge.clone())));
    tokio::time::timeout(Duration::from_secs(10), async {
        while judge.calls.load(Ordering::SeqCst) == 0 {
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
    assert!(detail.items[0].has_output);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Cancelled
    );
    assert_eq!(judge.calls.load(Ordering::SeqCst), 1);
    assert_eq!(judge.released.load(Ordering::SeqCst), 1);
    db.close().await;
}
