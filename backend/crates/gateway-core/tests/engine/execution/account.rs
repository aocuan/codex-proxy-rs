//! 管理后台指定账号执行的当前授权、预算、身份与取消收尾测试

use super::*;
use gateway_core::{
    engine::execution::StartAccountExecution,
    lifecycle::{CancellationToken, Deadline},
};

fn account_request() -> StartAccountExecution {
    StartAccountExecution {
        client_key_id: ClientApiKeyId::new("key_start_test").unwrap(),
        account_id: ProviderAccountId::new("acct_start").unwrap(),
        public_model: PublicModelId::new("gpt-start").unwrap(),
        operation: start_operation(),
        metadata: ExecutionRequestMetadata {
            protocol: "openai".to_owned(),
            endpoint: "/internal/evaluations".to_owned(),
            transport: ClientTransport::HttpJson,
            stream: false,
            client_ip: Some("127.0.0.1".parse().unwrap()),
            user_agent: Some("not-an-external-client".to_owned()),
            previous_response_id: None,
        },
        deadline_at: SystemTime::now() + Duration::from_secs(600),
        cancellation: CancellationToken::new(),
    }
}

#[derive(Default)]
struct PinnedProvider {
    contexts: Mutex<Vec<(ProviderAccountId, Deadline)>>,
}

#[async_trait]
impl Provider for PinnedProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn catalog_generation(&self) -> ProviderCatalogGeneration {
        ProviderCatalogGeneration::default()
    }

    async fn query_model_capabilities(
        &self,
    ) -> Result<Vec<ProviderModelCapabilities>, ProviderError> {
        Ok(Vec::new())
    }

    async fn execute(
        self: Arc<Self>,
        request: ProviderRequest,
        context: AttemptContext,
    ) -> Result<ProviderStream, ProviderError> {
        assert!(!context.is_diagnostic_required_account());
        self.contexts.lock().unwrap().push((
            context
                .required_account()
                .expect("required account")
                .clone(),
            context.deadline(),
        ));
        Arc::new(ChargedProvider::default())
            .execute(request, context)
            .await
    }
}

#[test]
fn account_execution_pins_account_records_internal_origin_and_settles_once() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let budget = Arc::new(Budget {
            active: admissions.active.clone(),
            ..Default::default()
        });
        let store = Arc::new(TrackingExecutionStore::default());
        let provider = Arc::new(PinnedProvider::default());
        let service = DefaultExecutionService::new(
            RuntimeSnapshotHandle::new(start_snapshot()),
            store.clone(),
            ProviderRegistry::new([provider.clone() as Arc<dyn Provider>]).unwrap(),
            admissions.clone(),
            Arc::new(UnusedContinuation),
            Arc::new(RecordingClientApiKeyUsage::default()),
            Arc::new(crate::RecordingDiagnostics::default()),
        )
        .with_budget(budget.clone());
        let request = account_request();
        let expected_account = request.account_id.clone();
        let expected_deadline = request.deadline_at;
        let mut started = service.start_account_execution(request).await.unwrap();
        started.session.collect_uncommitted().await.unwrap();
        started.session.commit_downstream(Some(200)).await.unwrap();

        assert_cleanup_completed(&admissions, &budget, &started.request_id);
        assert_eq!(
            *provider.contexts.lock().unwrap(),
            vec![(expected_account, Deadline::from(expected_deadline))]
        );
        let records = store.requests.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].client_transport, "internal_admin");
        assert_eq!(records[0].endpoint, "/internal/evaluations");
        assert!(records[0].client_ip.is_none());
        assert!(records[0].user_agent.is_none());
    });
}

#[test]
fn account_execution_enforces_key_budget_before_provider_dispatch() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let budget = Arc::new(Budget {
            reject: true,
            active: admissions.active.clone(),
            ..Default::default()
        });
        let service = service(admissions.clone(), budget.clone());
        let result = service.start_account_execution(account_request()).await;
        assert!(
            matches!(result, Err(error) if error.client_error_code() == Some("key_daily_budget_exceeded"))
        );
        assert!(!admissions.active.load(Ordering::SeqCst));
        assert!(budget.charges.lock().unwrap().is_empty());
    });
}

#[test]
fn account_execution_rejects_invalid_scope_or_identity_before_admission() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let service = service(admissions.clone(), Arc::default());
        for (key, account, expected) in [
            ("key_missing", "acct_start", GatewayErrorKind::Unauthorized),
            (
                "key_start_test",
                "acct_other",
                GatewayErrorKind::PolicyDenied,
            ),
        ] {
            let mut request = account_request();
            request.client_key_id = ClientApiKeyId::new(key).unwrap();
            request.account_id = ProviderAccountId::new(account).unwrap();
            let result = service.start_account_execution(request).await;
            assert!(matches!(result, Err(error) if error.kind() == expected));
        }
        assert!(admissions.limits.lock().unwrap().is_empty());
    });
}

#[test]
fn account_execution_revalidates_key_after_configuration_changes() {
    block_on(async {
        let snapshots = RuntimeSnapshotHandle::new(start_snapshot());
        let admissions = Arc::new(Admissions::default());
        let service = DefaultExecutionService::new(
            snapshots.clone(),
            Arc::new(TrackingExecutionStore::default()),
            ProviderRegistry::default(),
            admissions.clone(),
            Arc::new(UnusedContinuation),
            Arc::new(RecordingClientApiKeyUsage::default()),
            Arc::new(crate::RecordingDiagnostics::default()),
        );
        let started = service
            .start_account_execution(account_request())
            .await
            .unwrap();
        started.session.detach_finalize().await;
        snapshots.publish(start_snapshot_with_policy(
            2,
            false,
            RateLimits::unlimited(),
            FastMode::Default,
        ));
        let result = service.start_account_execution(account_request()).await;
        // 发布快照不包含停用 Key，内部入口与普通认证一致地视为不可认证
        assert!(matches!(result, Err(error) if error.kind() == GatewayErrorKind::Unauthorized));
        assert_eq!(admissions.limits.lock().unwrap().len(), 1);
    });
}

#[test]
fn account_execution_does_not_admit_cancelled_or_expired_tasks() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let service = service(admissions.clone(), Arc::default());
        for expected in [GatewayErrorKind::Cancelled, GatewayErrorKind::Timeout] {
            let mut request = account_request();
            if expected == GatewayErrorKind::Cancelled {
                request.cancellation.cancel();
            } else {
                request.deadline_at = SystemTime::UNIX_EPOCH;
            }
            let result = service.start_account_execution(request).await;
            assert!(matches!(result, Err(error) if error.kind() == expected));
        }
        assert!(admissions.limits.lock().unwrap().is_empty());
    });
}

#[test]
fn account_task_cancellation_finalizes_known_cost_and_releases_admission() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let budget = Arc::new(Budget {
            active: admissions.active.clone(),
            ..Default::default()
        });
        let service = charged_service(admissions.clone(), Some(budget.clone()), false);
        let request = account_request();
        let cancellation = request.cancellation.clone();
        let mut started = service.start_account_execution(request).await.unwrap();
        consume_charged_prefix(started.session.as_mut()).await;
        cancellation.cancel();
        assert!(matches!(
            started.session.next_event().await,
            Err(EngineError::Cancelled)
        ));
        let request_id = started.request_id.clone();
        started.session.detach_finalize().await;
        assert_cleanup_completed(&admissions, &budget, &request_id);
    });
}

#[derive(Default)]
struct LimitedCollector {
    reject_cost: bool,
    events: usize,
}

impl gateway_core::engine::execution::UncommittedEventCollector for LimitedCollector {
    fn clear(&mut self) {
        self.events = 0;
    }

    fn accept(
        &mut self,
        events: Vec<ProviderEvent>,
    ) -> Result<(), gateway_core::engine::execution::CollectionLimitExceeded> {
        for event in events {
            if self.reject_cost
                && event
                    .into_parts()
                    .0
                    .iter()
                    .any(|fact| matches!(fact, GatewayEvent::ProviderCost(_)))
            {
                return Err(gateway_core::engine::execution::CollectionLimitExceeded);
            }
            self.events += 1;
        }
        Ok(())
    }
}

#[test]
fn bounded_collection_rejection_settles_known_cost_without_downstream_commit() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let budget = Arc::new(Budget {
            active: admissions.active.clone(),
            ..Default::default()
        });
        let service = charged_service(admissions.clone(), Some(budget.clone()), false);
        let mut started = service
            .start_account_execution(account_request())
            .await
            .unwrap();
        let mut collector = LimitedCollector {
            reject_cost: true,
            ..Default::default()
        };
        assert!(matches!(
            started
                .session
                .collect_uncommitted_into(&mut collector)
                .await,
            Err(EngineError::CollectionLimitExceeded)
        ));
        assert!(started.session.is_finalized());
        assert!(started.session.commit_downstream(Some(200)).await.is_err());
        assert_cleanup_completed(&admissions, &budget, &started.request_id);
    });
}

#[test]
fn bounded_collection_leaves_commit_to_the_consumer_after_complete_output() {
    block_on(async {
        let admissions = Arc::new(Admissions::default());
        let budget = Arc::new(Budget {
            active: admissions.active.clone(),
            ..Default::default()
        });
        let service = charged_service(admissions.clone(), Some(budget.clone()), false);
        let mut started = service
            .start_account_execution(account_request())
            .await
            .unwrap();
        let mut collector = LimitedCollector::default();
        started
            .session
            .collect_uncommitted_into(&mut collector)
            .await
            .unwrap();
        assert_eq!(collector.events, 3);
        assert!(admissions.active.load(Ordering::SeqCst));
        assert_eq!(budget.settlements.load(Ordering::SeqCst), 0);
        started.session.commit_downstream(Some(200)).await.unwrap();
        assert_cleanup_completed(&admissions, &budget, &started.request_id);
    });
}
