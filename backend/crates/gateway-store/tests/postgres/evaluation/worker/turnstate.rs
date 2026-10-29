//! 测票复用持久队列、停止生命周期且不生成能力分数或模型账单

use super::*;
use gateway_admin::{
    model::{observability::DashboardWireProfile, provider_credentials::*},
    ports::provider::{
        ProviderAdmin, ProviderAdminError, ProviderAdminErrorKind, ProviderAdminRegistry,
    },
};
use gateway_core::{account::ProviderAccountId, routing::ProviderKind};

struct ProbeProvider {
    kind: ProviderKind,
    pending: bool,
    calls: Arc<AtomicUsize>,
    cleaned: Arc<AtomicUsize>,
}
fn unsupported_provider() -> ProviderAdminError {
    ProviderAdminError::new(ProviderAdminErrorKind::Unsupported)
}
#[async_trait::async_trait]
impl ProviderAdmin for ProbeProvider {
    async fn probe_turn_state(
        &self,
        _: &ProviderAccountId,
        _: &gateway_core::routing::UpstreamModelId,
        _: Option<&gateway_core::account::OpaqueProviderData>,
        cancellation: CancellationToken,
    ) -> Result<TurnStateProbeResult, ProviderAdminError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.pending {
            cancellation.cancelled().await;
        }
        self.cleaned.fetch_add(1, Ordering::SeqCst);
        Ok(TurnStateProbeResult::new(
            vec![
                TurnStateStep {
                    status: 200,
                    duration_ms: 100,
                    ticket_length: 10,
                    ticket_fingerprint: "same".into(),
                    route_changed: false,
                    completed: true
                };
                2
            ],
            if self.pending {
                Some("测票已取消".into())
            } else {
                None
            },
        ))
    }

    fn provider_kind(&self) -> &ProviderKind {
        &self.kind
    }

    async fn account_unavailable(&self, _: &ProviderAccountId) {}

    async fn connection_test_operation(
        &self,
        _: &gateway_core::routing::UpstreamModelId,
        _: &str,
    ) -> Result<gateway_core::operation::Operation, ProviderAdminError> {
        Err(unsupported_provider())
    }

    fn dashboard_wire_profile(&self) -> Option<DashboardWireProfile> {
        None
    }

    fn calculated_billing(
        &self,
        _: &gateway_admin::model::observability::ProviderBillingInput,
    ) -> Result<
        Option<gateway_admin::model::observability::CalculatedBillingBreakdown>,
        ProviderAdminError,
    > {
        Ok(None)
    }

    async fn prepare_import(
        &self,
        _: PrepareCredentialImport,
    ) -> Result<PreparedCredentialImport, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn start_authorization(
        &self,
        _: gateway_admin::model::provider_credentials::PendingAuthorizationMutation,
    ) -> Result<AuthorizationStarted, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn complete_authorization(
        &self,
        _: CompleteAuthorization,
    ) -> Result<PreparedAuthorizationCommit, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn prepare_rotation(
        &self,
        _: PrepareCredentialRotation,
    ) -> Result<PreparedCredentialRotation, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn prepare_refresh(
        &self,
        _: PrepareCredentialRefresh,
    ) -> Result<PreparedCredentialRotation, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn quota(
        &self,
        _: gateway_admin::model::provider_credentials::ProviderQuotaRequest,
    ) -> Result<ProviderQuota, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn models(
        &self,
        _: &ProviderAccountId,
        _: bool,
    ) -> Result<ProviderModels, ProviderAdminError> {
        Err(unsupported_provider())
    }

    async fn export_credentials(
        &self,
        _: Vec<ProviderExportCredentialInput>,
    ) -> Result<ProviderExport, ProviderAdminError> {
        Err(unsupported_provider())
    }
}

#[tokio::test]
async fn ticket_probe_keeps_route_results_out_of_ability_scores_and_billing() {
    let Some(db) = TestDatabase::create("eval_ticket").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Answer).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let cleaned = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: calls.clone(),
        cleaned,
    });
    let task = task
        .with_turnstate(ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap());
    let mut params = parameters(&["acct_right", "acct_wrong"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.definition = None;
    params.reasoning_effort = Some("low".into());
    let run = service
        .submit(uuid::Uuid::new_v4(), params, &context())
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.run.succeeded_items, 2);
    assert_eq!(detail.run.correct_items, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(execution.accounts.lock().unwrap().is_empty());
    for item in detail.items {
        assert_eq!(item.verdict, EvaluationVerdict::Ungraded);
        assert!(item.judgment.is_none() && item.request_id.is_none() && item.has_output);
        assert!(item.observation.cost_amount.is_none());
        assert_eq!(
            item.observation.turn_state.as_ref().unwrap().outcome,
            TurnStateOutcome::Accepted
        );
        assert!(repository.load_output(&item.id).await.unwrap().is_some());
    }
    db.close().await;
}

#[tokio::test]
async fn stopping_ticket_probe_awaits_network_cleanup_and_does_not_replay_it() {
    let Some(db) = TestDatabase::create("eval_ticket_stop").await else {
        return;
    };
    let (repository, _, task, service) = setup(&db, Reply::Answer).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let cleaned = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: true,
        calls: calls.clone(),
        cleaned: cleaned.clone(),
    });
    let task = task
        .with_turnstate(ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap());
    let mut params = parameters(&["acct_right"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let run = service
        .submit(uuid::Uuid::new_v4(), params, &context())
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    tokio::time::timeout(Duration::from_secs(10), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    repository.stop_run(&run.id, &context()).await.unwrap();
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Cancelled);
    assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    assert!(repository.claim_next(Utc::now()).await.unwrap().is_none());
    db.close().await;
}
