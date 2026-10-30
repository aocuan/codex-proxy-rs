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
    preference_fixture: Option<Arc<std::sync::atomic::AtomicBool>>,
    gate: Option<Arc<tokio::sync::Notify>>,
}
fn unsupported_provider() -> ProviderAdminError {
    ProviderAdminError::new(ProviderAdminErrorKind::Unsupported)
}

#[tokio::test]
async fn account_result_filters_share_history_conditions_and_apply_before_directory_pagination() {
    use gateway_admin::{
        model::{
            PageSize,
            accounts::{AccountGroupFilter, AccountListQuery},
        },
        ports::store::AccountStore,
    };
    let Some(db) = TestDatabase::create("eval_account_filter").await else {
        return;
    };
    let (repository, _, _, service) = setup(&db, Reply::Answer).await;
    sqlx::query("insert into provider_accounts (id,provider_kind,name,authentication_kind,provider_credentials_json,has_refresh_token,credential_observed_at,created_at,updated_at)
        select id,'openai',id,'oauth','{}'::jsonb,false,now(),now(),now() from unnest($1::text[]) id")
        .bind(vec!["acct_unknown", "acct_failed", "acct_empty", "acct_legacy"])
        .execute(&db.pool).await.unwrap();
    sqlx::query(
        "update provider_accounts set authentication_kind='oauth' where id <> 'acct_judge'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: Arc::default(),
        cleaned: Arc::default(),
        preference_fixture: None,
        gate: None,
    });
    let service = service
        .with_turnstate(ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap());
    let probe_parameters = |ids: &[&str]| {
        let mut params = parameters(ids, 1);
        params.kind = EvaluationKind::Turnstate;
        params.reasoning_effort = Some("low".into());
        params
    };
    let mut old = probe_parameters(&["acct_legacy"]);
    old.model = PublicModelId::new("old-filter-model").unwrap();
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(old, "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    let old = repository.claim_next(Utc::now()).await.unwrap().unwrap();
    repository
        .save_output(
            &old.item.id,
            EvaluationOutput {
                response_text: "old".into(),
                artifact_html: None,
            },
        )
        .await
        .unwrap();
    repository
        .finish_item(
            &old.item.id,
            super::super::transactions::completion(EvaluationItemStatus::Succeeded),
        )
        .await
        .unwrap();

    let accounts = [
        "acct_right",
        "acct_target",
        "acct_wrong",
        "acct_unknown",
        "acct_failed",
        "acct_judge",
    ];
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(probe_parameters(&accounts), "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    for _ in &accounts {
        let claim = repository.claim_next(Utc::now()).await.unwrap().unwrap();
        repository
            .save_output(
                &claim.item.id,
                EvaluationOutput {
                    response_text: "probe".into(),
                    artifact_html: None,
                },
            )
            .await
            .unwrap();
        let status = if claim.item.account_id == "acct_failed" {
            EvaluationItemStatus::Failed
        } else {
            EvaluationItemStatus::Succeeded
        };
        let outcome = match claim.item.account_id.as_str() {
            "acct_wrong" => TurnStateOutcome::Refreshed,
            "acct_unknown" => TurnStateOutcome::Unknown,
            _ => TurnStateOutcome::Accepted,
        };
        let mut completion = super::super::transactions::completion(status);
        completion.observation.turn_state = Some(TurnStateProbeResult {
            outcome,
            steps: Vec::new(),
            error: None,
        });
        repository
            .finish_item(&claim.item.id, completion)
            .await
            .unwrap();
    }
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(
                probe_parameters(&["acct_pending", "acct_right"]),
                "test",
                Utc::now(),
            ),
            &context(),
        )
        .await
        .unwrap();
    for (filter, expected) in [
        (
            EvaluationAccountStatus::Stable,
            vec!["acct_right", "acct_target"],
        ),
        (EvaluationAccountStatus::Changed, vec!["acct_wrong"]),
        (EvaluationAccountStatus::Unknown, vec!["acct_unknown"]),
        (EvaluationAccountStatus::Failed, vec!["acct_failed"]),
        (EvaluationAccountStatus::Pending, vec!["acct_pending"]),
        (
            EvaluationAccountStatus::NoRecord,
            vec!["acct_empty", "acct_legacy"],
        ),
        (EvaluationAccountStatus::NotApplicable, vec!["acct_judge"]),
    ] {
        assert_eq!(service.filter_account_ids(filter).await.unwrap(), expected);
    }
    let history = repository
        .account_history(&["acct_right".into()], 28, None)
        .await
        .unwrap();
    assert_eq!(history[0].status, EvaluationItemStatus::Queued);
    assert_eq!(history[1].turn_state, Some(TurnStateOutcome::Accepted));
    let matched = service
        .filter_account_ids(EvaluationAccountStatus::Stable)
        .await
        .unwrap();
    let store = super::super::super::admin_account_store(&db.pool);
    let query = AccountListQuery {
        page: 1,
        page_size: PageSize::new(1).unwrap(),
        provider_kind: None,
        group_filter: None,
        search: None,
        status: None,
        account_ids: Some(matched.clone()),
        sort: None,
    };
    let mut pages = Vec::new();
    for page in 1..=3 {
        let result = store
            .list_accounts(
                AccountListQuery {
                    page,
                    ..query.clone()
                },
                Default::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.summary.total, 9);
        assert_eq!(result.items.len(), usize::from(page < 3));
        pages.extend(result.items.into_iter().map(|item| item.account.id));
    }
    pages.sort();
    assert_eq!(pages, matched);
    let searched = store
        .list_accounts(
            AccountListQuery {
                search: Some("acct_wrong".into()),
                ..query.clone()
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(searched.total, 0);
    let empty = store
        .list_accounts(
            AccountListQuery {
                account_ids: Some(Vec::new()),
                ..query.clone()
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(empty.total, 0);
    assert!(empty.items.is_empty());
    let group = format!("grp_{}", "a".repeat(32));
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'filter group','#FFFFFFFF',now(),now())")
        .bind(&group).execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_target',now())")
        .bind(&group).execute(&db.pool).await.unwrap();
    let grouped = store
        .list_accounts(
            AccountListQuery {
                group_filter: Some(AccountGroupFilter::Group(
                    gateway_core::routing::AccountGroupId::new(group).unwrap(),
                )),
                ..query
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(grouped.total, 1);
    assert_eq!(grouped.items[0].account.id, "acct_target");

    let mut different_model = probe_parameters(&["acct_right"]);
    different_model.model = PublicModelId::new("new-filter-model").unwrap();
    repository
        .enqueue_manual(
            uuid::Uuid::new_v4(),
            EvaluationRunSeed::new(different_model, "test", Utc::now()),
            &context(),
        )
        .await
        .unwrap();
    assert!(
        service
            .filter_account_ids(EvaluationAccountStatus::Stable)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        service
            .filter_account_ids(EvaluationAccountStatus::Pending)
            .await
            .unwrap(),
        ["acct_right"]
    );
    db.close().await;
}

#[async_trait::async_trait]
impl ProviderAdmin for ProbeProvider {
    fn account_capabilities(
        &self,
        _: &ProviderAccountId,
        authentication_kind: &str,
    ) -> gateway_admin::model::accounts::ProviderAccountCapabilities {
        gateway_admin::model::accounts::ProviderAccountCapabilities {
            turn_state_probe: authentication_kind == "oauth",
            ..Default::default()
        }
    }

    async fn probe_turn_state(
        &self,
        account: &ProviderAccountId,
        _: &gateway_core::routing::UpstreamModelId,
        _: Option<&gateway_core::account::OpaqueProviderData>,
        cancellation: CancellationToken,
    ) -> Result<TurnStateProbeResult, ProviderAdminError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.pending {
            cancellation.cancelled().await;
        }
        if let Some(gate) = &self.gate {
            gate.notified().await;
        }
        self.cleaned.fetch_add(1, Ordering::SeqCst);
        let step = TurnStateStep {
            status: 200,
            duration_ms: 100,
            ticket_length: 10,
            ticket_fingerprint: "same".into(),
            route_changed: false,
            completed: true,
        };
        let mut steps = vec![step.clone(), step];
        if let Some(recovered) = &self.preference_fixture {
            if account.as_str() == "acct_pending" {
                steps.pop();
            } else if account.as_str() == "acct_wrong" && !recovered.load(Ordering::SeqCst) {
                steps[1].ticket_fingerprint = "changed".into();
            }
        }
        Ok(TurnStateProbeResult::new(
            steps,
            if self.pending {
                Some("测票已取消".into())
            } else if self.preference_fixture.is_some() && account.as_str() == "acct_target" {
                Some("模拟测票失败".into())
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

struct PreferencePublisher {
    handle: gateway_core::runtime::RuntimeSnapshotHandle,
    revisions: Mutex<Vec<u64>>,
    deny_at: Option<u64>,
}

fn preference_snapshot(revision: u64, enabled: bool) -> gateway_core::routing::RuntimeSnapshot {
    use gateway_core::account::scope::{
        ClientRoutingScope, FrozenAccountScope, RuntimeAccount, RuntimeAccountDirectory,
    };
    let provider = ProviderKind::new("openai").unwrap();
    let accounts = ["acct_right", "acct_wrong", "acct_pending", "acct_target"]
        .into_iter()
        .map(|id| {
            (
                ProviderAccountId::new(id).unwrap(),
                RuntimeAccount::new(provider.clone(), Default::default()),
            )
        })
        .collect();
    let directory = Arc::new(RuntimeAccountDirectory::new(accounts));
    let policy = gateway_core::policy::ClientPolicy::new(
        gateway_core::policy::ClientApiKeyId::new("key_evaluation").unwrap(),
        gateway_core::policy::PlaintextClientApiKey::new("test-only").unwrap(),
        Arc::new(FrozenAccountScope::new(
            directory,
            ClientRoutingScope::all_accounts(),
        )),
        enabled,
        Default::default(),
    );
    gateway_core::routing::RuntimeSnapshot::new(
        gateway_core::routing::ConfigRevision::new(revision).unwrap(),
        gateway_core::settings::SettingsValues::new(1, "smart", Default::default(), None, None),
        vec![provider],
        vec![],
        vec![policy],
    )
    .unwrap()
}

impl gateway_core::runtime::SnapshotControl for PreferencePublisher {
    fn publish_committed(
        &self,
        revision: gateway_core::routing::ConfigRevision,
    ) -> BoxFuture<'_, ()> {
        let mut revisions = self.revisions.lock().unwrap();
        revisions.push(revision.get());
        if self
            .handle
            .revision()
            .is_none_or(|current| current.get() < revision.get())
        {
            let enabled = self.deny_at.is_none_or(|denied| revision.get() < denied);
            self.handle
                .publish(preference_snapshot(revision.get(), enabled));
        }
        Box::pin(async {})
    }
}

#[tokio::test]
async fn preference_plan_moves_only_explicit_results_and_recovers_on_the_next_round() {
    let Some(db) = TestDatabase::create("eval_preference_worker").await else {
        return;
    };
    let (repository, execution, _, service) = setup(&db, Reply::Answer).await;
    let source = format!("grp_{}", "a".repeat(32));
    let target = format!("grp_{}", "b".repeat(32));
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id in ('acct_right','acct_wrong','acct_pending','acct_target')")
        .execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'source','#FFFFFFFF',now(),now()),($2,'preferred','#FFFFFFFF',now(),now())")
        .bind(&source).bind(&target).execute(&db.pool).await.unwrap();
    sqlx::query("insert into client_api_keys (id,name,key,enabled,created_at,updated_at) values ('key_evaluation','test','evaluation-test-key',true,now(),now())")
        .execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_right',now()),($1,'acct_wrong',now()),($1,'acct_pending',now()),($1,'acct_target',now()),($2,'acct_wrong',now()),($2,'acct_pending',now()),($2,'acct_target',now())")
        .bind(&source).bind(&target).execute(&db.pool).await.unwrap();
    let recovered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: Arc::default(),
        cleaned: Arc::default(),
        preference_fixture: Some(recovered.clone()),
        gate: None,
    });
    let registry = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let snapshot = gateway_core::runtime::RuntimeSnapshotHandle::new(preference_snapshot(1, true));
    let publisher = Arc::new(PreferencePublisher {
        handle: snapshot.clone(),
        revisions: Mutex::default(),
        deny_at: None,
    });
    let task = EvaluationTask::new(
        repository.clone(),
        execution,
        Arc::new(admin_observability_store(&db.pool)),
        "test".into(),
        Default::default(),
        snapshot,
    )
    .with_turnstate(registry.clone())
    .with_snapshot_control(publisher.clone());
    let service = service.with_turnstate(registry);
    let mut params = parameters(&[], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let plan = service
        .save_plan(
            None,
            None,
            EvaluationPlanSettings {
                account_preference: Some(EvaluationAccountPreference {
                    group_id: target.clone(),
                }),
                account_selection: EvaluationAccountSelection::Groups {
                    group_ids: vec![source.clone()],
                },
                name: "自动优选".into(),
                parameters: params,
                enabled: true,
                interval_minutes: Some(1),
                cron_expression: String::new(),
                timezone: Default::default(),
            },
            &context(),
        )
        .await
        .unwrap();
    task.advance_schedules(plan.next_run_at.unwrap())
        .await
        .unwrap();
    let run = repository.list_runs(100, 0).await.unwrap().remove(0);
    let (cancel, handle) = start(task);
    let detail = wait_finished(&repository, &run.id).await;
    let outcomes: std::collections::BTreeMap<_, _> = detail
        .items
        .iter()
        .map(|item| {
            (
                item.account_id.as_str(),
                item.observation.account_preference.as_ref().unwrap().status,
            )
        })
        .collect();
    assert_eq!(
        outcomes.get("acct_right"),
        Some(&EvaluationPreferenceStatus::Joined)
    );
    assert_eq!(
        outcomes.get("acct_wrong"),
        Some(&EvaluationPreferenceStatus::Removed)
    );
    assert_eq!(
        outcomes.get("acct_pending"),
        Some(&EvaluationPreferenceStatus::Skipped)
    );
    assert_eq!(
        outcomes.get("acct_target"),
        Some(&EvaluationPreferenceStatus::Skipped)
    );
    let members: Vec<String> = sqlx::query_scalar("select provider_account_id from account_group_accounts where account_group_id=$1 order by provider_account_id")
        .bind(&target).fetch_all(&db.pool).await.unwrap();
    assert_eq!(members, ["acct_pending", "acct_right", "acct_target"]);
    let source_count: i64 =
        sqlx::query_scalar("select count(*) from account_group_accounts where account_group_id=$1")
            .bind(&source)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(source_count, 4);
    let audit_count: i64 = sqlx::query_scalar("select count(*) from admin_audit_events where action in ('auto_group_join','auto_group_remove') and config_revision is not null")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(audit_count, 4);
    let paired_audits: i64 = sqlx::query_scalar(
        "select count(*) from (select admin_request_id, action from admin_audit_events
         where action in ('auto_group_join','auto_group_remove')
         group by admin_request_id, action having count(distinct entity_kind)=2) paired",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(paired_audits, 2);
    assert!(publisher.revisions.lock().unwrap().len() >= 2);

    recovered.store(true, Ordering::SeqCst);
    sqlx::query("update evaluation_plans set next_run_at=now() where id=$1")
        .bind(&plan.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            let runs = repository.list_runs(100, 0).await.unwrap();
            if let Some(next) = runs.into_iter().find(|item| item.id != run.id) {
                break next;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let next = wait_finished(&repository, &next.id).await;
    assert_eq!(
        next.items
            .iter()
            .find(|item| item.account_id == "acct_wrong")
            .unwrap()
            .observation
            .account_preference
            .as_ref()
            .unwrap()
            .status,
        EvaluationPreferenceStatus::Joined
    );
    let restored: bool = sqlx::query_scalar("select exists(select 1 from account_group_accounts where account_group_id=$1 and provider_account_id='acct_wrong')")
        .bind(&target).fetch_one(&db.pool).await.unwrap();
    assert!(restored);
    cancel.cancel();
    handle.await.unwrap().unwrap();
    db.close().await;
}

#[tokio::test]
async fn preference_rechecks_key_policy_after_concurrent_configuration_change() {
    let Some(db) = TestDatabase::create("eval_preference_policy_race").await else {
        return;
    };
    let (repository, execution, _, service) = setup(&db, Reply::Answer).await;
    let target = format!("grp_{}", "f".repeat(32));
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id='acct_wrong'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'preferred','#FFFFFFFF',now(),now())")
        .bind(&target).execute(&db.pool).await.unwrap();
    sqlx::query("insert into client_api_keys (id,name,key,enabled,created_at,updated_at) values ('key_evaluation','test','evaluation-test-key',true,now(),now())")
        .execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_wrong',now())")
        .bind(&target).execute(&db.pool).await.unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: calls.clone(),
        cleaned: Arc::default(),
        preference_fixture: Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
        gate: Some(gate.clone()),
    });
    let registry = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let snapshot = gateway_core::runtime::RuntimeSnapshotHandle::new(preference_snapshot(1, true));
    let publisher = Arc::new(PreferencePublisher {
        handle: snapshot.clone(),
        revisions: Mutex::default(),
        deny_at: Some(2),
    });
    let task = EvaluationTask::new(
        repository.clone(),
        execution,
        Arc::new(admin_observability_store(&db.pool)),
        "test".into(),
        Default::default(),
        snapshot,
    )
    .with_turnstate(registry.clone())
    .with_snapshot_control(publisher.clone());
    let service = service.with_turnstate(registry);
    let mut params = parameters(&["acct_wrong"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let plan = service
        .save_plan(
            None,
            None,
            EvaluationPlanSettings {
                account_preference: Some(EvaluationAccountPreference {
                    group_id: target.clone(),
                }),
                account_selection: EvaluationAccountSelection::Fixed,
                name: "并发授权".into(),
                parameters: params,
                enabled: true,
                interval_minutes: Some(1),
                cron_expression: String::new(),
                timezone: Default::default(),
            },
            &context(),
        )
        .await
        .unwrap();
    task.advance_schedules(plan.next_run_at.unwrap())
        .await
        .unwrap();
    let run = repository.list_runs(100, 0).await.unwrap().remove(0);
    let (cancel, handle) = start(task);
    tokio::time::timeout(Duration::from_secs(10), async {
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    sqlx::query("update runtime_settings set config_revision=config_revision+1 where id=1")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("update client_api_keys set enabled=false where id='key_evaluation'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    gate.notify_one();
    let detail = wait_finished(&repository, &run.id).await;
    let item = &detail.items[0];
    assert_eq!(
        item.observation.turn_state.as_ref().unwrap().outcome,
        TurnStateOutcome::Refreshed
    );
    let preference = item.observation.account_preference.as_ref().unwrap();
    assert_eq!(preference.status, EvaluationPreferenceStatus::Skipped);
    assert!(preference.reason.as_deref().unwrap().contains("授权范围"));
    let still_member: bool = sqlx::query_scalar("select exists(select 1 from account_group_accounts where account_group_id=$1 and provider_account_id='acct_wrong')")
        .bind(&target).fetch_one(&db.pool).await.unwrap();
    assert!(still_member);
    assert!(publisher.revisions.lock().unwrap().contains(&2));
    cancel.cancel();
    handle.await.unwrap().unwrap();
    db.close().await;
}

#[tokio::test]
async fn stale_stopped_or_unauthorized_preference_results_preserve_group_membership() {
    for scenario in [
        "paused",
        "stopped",
        "key_changed",
        "group_disabled",
        "source_removed",
        "audit_failure",
    ] {
        let Some(db) = TestDatabase::create("eval_preference_guard").await else {
            return;
        };
        let (repository, _, _, _) = setup(&db, Reply::Answer).await;
        let source = format!("grp_{}", "c".repeat(32));
        let target = format!("grp_{}", "d".repeat(32));
        sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'source','#FFFFFFFF',now(),now()),($2,'preferred','#FFFFFFFF',now(),now())")
            .bind(&source).bind(&target).execute(&db.pool).await.unwrap();
        sqlx::query("insert into client_api_keys (id,name,key,enabled,created_at,updated_at) values ('key_evaluation','test','evaluation-test-key',true,now(),now())")
            .execute(&db.pool).await.unwrap();
        sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_right',now())")
            .bind(&source).execute(&db.pool).await.unwrap();
        let mut params = parameters(&[], 1);
        params.kind = EvaluationKind::Turnstate;
        params.reasoning_effort = Some("low".into());
        let settings = EvaluationPlanSettings {
            account_preference: Some(EvaluationAccountPreference {
                group_id: target.clone(),
            }),
            account_selection: EvaluationAccountSelection::Groups {
                group_ids: vec![source.clone()],
            },
            name: "guard".into(),
            parameters: params.clone(),
            enabled: true,
            interval_minutes: Some(1),
            cron_expression: String::new(),
            timezone: Default::default(),
        };
        let now = Utc::now();
        let plan = repository
            .save_plan(
                SaveEvaluationPlan {
                    id: "eval_plan_guard".into(),
                    expected_revision: None,
                    settings: settings.clone(),
                    next_run_at: Some(now),
                },
                &context(),
            )
            .await
            .unwrap();
        params.account_ids = vec!["acct_right".into()];
        let mut seed = EvaluationRunSeed::new(params, "test", now);
        seed.profile.account_selection = settings.account_selection.clone();
        seed.profile.account_preference = Some(EvaluationPreferenceSnapshot {
            group_id: target.clone(),
            plan_revision: plan.revision,
        });
        repository
            .enqueue_scheduled(EnqueueScheduledEvaluation {
                plan_id: plan.id.clone(),
                expected_revision: plan.revision,
                scheduled_at: now,
                next_run_at: now + chrono::Duration::minutes(1),
                seed,
                skip_reason: None,
            })
            .await
            .unwrap();
        let claimed = repository.claim_next(now).await.unwrap().unwrap();
        repository
            .save_output(
                &claimed.item.id,
                EvaluationOutput {
                    response_text: "saved".into(),
                    artifact_html: None,
                },
            )
            .await
            .unwrap();
        match scenario {
            "paused" => {
                repository
                    .save_plan(
                        SaveEvaluationPlan {
                            id: plan.id.clone(),
                            expected_revision: Some(plan.revision),
                            settings: EvaluationPlanSettings {
                                enabled: false,
                                ..settings.clone()
                            },
                            next_run_at: None,
                        },
                        &context(),
                    )
                    .await
                    .unwrap();
            }
            "stopped" => {
                repository
                    .stop_run(&claimed.item.run_id, &context())
                    .await
                    .unwrap();
            }
            "key_changed" => {
                sqlx::query("insert into client_api_key_groups (client_api_key_id,account_group_id,created_at) values ('key_evaluation',$1,now())").bind(&target).execute(&db.pool).await.unwrap();
                sqlx::query(
                    "update runtime_settings set config_revision=config_revision+1 where id=1",
                )
                .execute(&db.pool)
                .await
                .unwrap();
            }
            "group_disabled" => {
                sqlx::query("update account_groups set enabled=false where id=$1")
                    .bind(&target)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            "source_removed" => {
                sqlx::query("delete from account_group_accounts where account_group_id=$1 and provider_account_id='acct_right'").bind(&source).execute(&db.pool).await.unwrap();
            }
            "audit_failure" => {
                sqlx::query("alter table admin_audit_events add constraint reject_auto_group check (entity_kind <> 'account_group' or action not in ('auto_group_join','auto_group_remove'))").execute(&db.pool).await.unwrap();
            }
            _ => unreachable!(),
        }
        let mut completion =
            super::super::transactions::completion(EvaluationItemStatus::Succeeded);
        completion.account_preference = Some(EvaluationPreferenceDecision {
            rule: claimed.profile.account_preference.clone().unwrap(),
            desired_member: Some(true),
            expected_revision: Some(gateway_admin::model::Revision::new(1).unwrap()),
            skip_reason: None,
        });
        if scenario == "key_changed" {
            let stale = repository
                .finish_item(&claimed.item.id, completion.clone())
                .await
                .unwrap();
            assert!(stale.retry_preference);
            assert_eq!(stale.config_revision.unwrap().get(), 2);
            let decision = completion.account_preference.as_mut().unwrap();
            decision.desired_member = None;
            decision.skip_reason = Some("检测 Key 的授权范围已变化".into());
        }
        let result = repository.finish_item(&claimed.item.id, completion).await;
        if scenario == "audit_failure" {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().config_revision.is_none(), "{scenario}");
            let detail = repository
                .load_run(&claimed.item.run_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                detail.items[0]
                    .observation
                    .account_preference
                    .as_ref()
                    .unwrap()
                    .status,
                EvaluationPreferenceStatus::Skipped,
                "{scenario}"
            );
        }
        let member: bool = sqlx::query_scalar("select exists(select 1 from account_group_accounts where account_group_id=$1 and provider_account_id='acct_right')")
            .bind(&target).fetch_one(&db.pool).await.unwrap();
        assert!(!member, "{scenario}");
        if scenario == "audit_failure" {
            let detail = repository
                .load_run(&claimed.item.run_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(detail.items[0].status, EvaluationItemStatus::Running);
            let revision: i64 =
                sqlx::query_scalar("select config_revision from runtime_settings where id=1")
                    .fetch_one(&db.pool)
                    .await
                    .unwrap();
            assert_eq!(revision, 1);
            let audit_count: i64 = sqlx::query_scalar("select count(*) from admin_audit_events where action in ('auto_group_join','auto_group_remove')")
                .fetch_one(&db.pool).await.unwrap();
            assert_eq!(audit_count, 0);
        }
        db.close().await;
    }
}

#[tokio::test]
async fn a_preference_group_can_have_only_one_enabled_plan() {
    let Some(db) = TestDatabase::create("eval_preference_owner").await else {
        return;
    };
    let (repository, _, _, _) = setup(&db, Reply::Answer).await;
    let target = format!("grp_{}", "e".repeat(32));
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'preferred','#FFFFFFFF',now(),now())")
        .bind(&target).execute(&db.pool).await.unwrap();
    let mut params = parameters(&["acct_right"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let settings = EvaluationPlanSettings {
        account_preference: Some(EvaluationAccountPreference { group_id: target }),
        account_selection: EvaluationAccountSelection::Fixed,
        name: "优选".into(),
        parameters: params,
        enabled: true,
        interval_minutes: Some(1),
        cron_expression: String::new(),
        timezone: Default::default(),
    };
    let save = |id: &str| SaveEvaluationPlan {
        id: id.into(),
        expected_revision: None,
        settings: settings.clone(),
        next_run_at: Some(Utc::now()),
    };
    let first = repository
        .save_plan(save("eval_plan_first"), &context())
        .await
        .unwrap();
    assert!(
        repository
            .save_plan(save("eval_plan_second"), &context())
            .await
            .is_err()
    );
    repository
        .save_plan(
            SaveEvaluationPlan {
                id: first.id,
                expected_revision: Some(first.revision),
                settings: EvaluationPlanSettings {
                    enabled: false,
                    ..settings.clone()
                },
                next_run_at: None,
            },
            &context(),
        )
        .await
        .unwrap();
    assert!(
        repository
            .save_plan(save("eval_plan_second"), &context())
            .await
            .is_ok()
    );
    db.close().await;
}

#[tokio::test]
async fn ticket_probe_filters_api_keys_before_manual_and_fixed_work() {
    let Some(db) = TestDatabase::create("eval_ticket_filter").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Answer).await;
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id='acct_right'")
        .execute(&db.pool)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: calls.clone(),
        cleaned: Arc::default(),
        preference_fixture: None,
        gate: None,
    });
    let providers = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let service = service.with_turnstate(providers.clone());
    let task = task.with_turnstate(providers);
    let mut params = parameters(&["acct_right", "acct_wrong"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let plan = service
        .save_plan(
            None,
            None,
            EvaluationPlanSettings {
                account_preference: None,
                account_selection: EvaluationAccountSelection::Fixed,
                name: "仅保留支持的账号".into(),
                parameters: params.clone(),
                enabled: true,
                interval_minutes: Some(60),
                cron_expression: String::new(),
                timezone: Default::default(),
            },
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(plan.settings.parameters.account_ids, ["acct_right"]);
    let run = service
        .submit(uuid::Uuid::new_v4(), params, &context())
        .await
        .unwrap();
    assert_eq!(run.parameters.account_ids, ["acct_right"]);
    let (cancel, handle) = start(task);
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.run.total_items, 1);
    assert_eq!(detail.run.succeeded_items, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(execution.accounts.lock().unwrap().is_empty());
    let ability = service
        .submit(
            uuid::Uuid::new_v4(),
            parameters(&["acct_wrong"], 1),
            &context(),
        )
        .await
        .unwrap();
    assert_eq!(ability.parameters.account_ids, ["acct_wrong"]);
    db.close().await;
}

#[tokio::test]
async fn api_key_only_ticket_requests_do_not_create_runs_or_enabled_plans() {
    let Some(db) = TestDatabase::create("eval_ticket_api_only").await else {
        return;
    };
    let (repository, _, _, service) = setup(&db, Reply::Answer).await;
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: Arc::default(),
        cleaned: Arc::default(),
        preference_fixture: None,
        gate: None,
    });
    let service = service
        .with_turnstate(ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap());
    let mut params = parameters(&["acct_wrong"], 1);
    params.kind = EvaluationKind::Turnstate;
    params.reasoning_effort = Some("low".into());
    let error = service
        .submit(uuid::Uuid::new_v4(), params.clone(), &context())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("没有支持快速探票的账号"));
    assert!(repository.list_runs(100, 0).await.unwrap().is_empty());
    let settings = EvaluationPlanSettings {
        account_preference: None,
        account_selection: EvaluationAccountSelection::Fixed,
        name: "API 不参加探票".into(),
        parameters: params,
        enabled: true,
        interval_minutes: Some(1),
        cron_expression: String::new(),
        timezone: Default::default(),
    };
    assert!(
        service
            .save_plan(None, None, settings.clone(), &context())
            .await
            .is_err()
    );
    assert!(repository.list_plans().await.unwrap().is_empty());
    let paused = service
        .save_plan(
            None,
            None,
            EvaluationPlanSettings {
                account_preference: None,
                enabled: false,
                ..settings
            },
            &context(),
        )
        .await
        .unwrap();
    assert!(paused.settings.parameters.account_ids.is_empty());
    assert!(!paused.settings.enabled);
    db.close().await;
}

#[tokio::test]
async fn scheduled_ticket_ranges_and_available_counts_use_provider_capabilities() {
    let Some(db) = TestDatabase::create("eval_ticket_schedule_filter").await else {
        return;
    };
    let (repository, _, task, service) = setup(&db, Reply::Answer).await;
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id='acct_right'")
        .execute(&db.pool)
        .await
        .unwrap();
    let group = format!("grp_{}", "a".repeat(32));
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'probe','#FFFFFFFF',now(),now())").bind(&group).execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_right',now()),($1,'acct_wrong',now())").bind(&group).execute(&db.pool).await.unwrap();
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: Arc::default(),
        cleaned: Arc::default(),
        preference_fixture: None,
        gate: None,
    });
    let providers = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let task = task.with_turnstate(providers.clone());
    let service = service.with_turnstate(providers);
    let now = Utc::now();
    for (name, selection) in [
        ("all", EvaluationAccountSelection::All),
        (
            "group",
            EvaluationAccountSelection::Groups {
                group_ids: vec![group.clone()],
            },
        ),
        ("fixed", EvaluationAccountSelection::Fixed),
    ] {
        let mut params = parameters(
            if selection.is_fixed() {
                &["acct_right", "acct_wrong"]
            } else {
                &[]
            },
            1,
        );
        params.kind = EvaluationKind::Turnstate;
        params.reasoning_effort = Some("low".into());
        let id = format!("eval_plan_probe_{name}");
        repository
            .save_plan(
                SaveEvaluationPlan {
                    id: id.clone(),
                    expected_revision: None,
                    settings: EvaluationPlanSettings {
                        account_preference: None,
                        account_selection: selection,
                        name: name.into(),
                        parameters: params,
                        enabled: true,
                        interval_minutes: Some(1),
                        cron_expression: String::new(),
                        timezone: Default::default(),
                    },
                    next_run_at: Some(now),
                },
                &context(),
            )
            .await
            .unwrap();
        task.advance_schedules(now).await.unwrap();
        let run = repository
            .list_runs(100, 0)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.plan_id.as_deref() == Some(id.as_str()))
            .unwrap();
        assert_eq!(run.parameters.account_ids, ["acct_right"]);
    }
    let plans = service.list_plans().await.unwrap();
    assert_eq!(plans.len(), 3);
    assert!(plans.iter().all(|p| p.available_accounts == Some(1)));
    let fixed = plans.iter().find(|p| p.id.ends_with("fixed")).unwrap();
    assert_eq!(fixed.settings.parameters.account_ids, ["acct_right"]);
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id='acct_wrong'")
        .execute(&db.pool)
        .await
        .unwrap();
    let plans = service.list_plans().await.unwrap();
    assert!(
        plans
            .iter()
            .filter(|p| !p.settings.account_selection.is_fixed())
            .all(|p| p.available_accounts == Some(2))
    );
    db.close().await;
}

#[tokio::test]
async fn ticket_probe_keeps_route_results_out_of_ability_scores_and_billing() {
    let Some(db) = TestDatabase::create("eval_ticket").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::Answer).await;
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id in ('acct_right','acct_wrong')")
        .execute(&db.pool).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let cleaned = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: false,
        calls: calls.clone(),
        cleaned,
        preference_fixture: None,
        gate: None,
    });
    let providers = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let task = task.with_turnstate(providers.clone());
    let service = service.with_turnstate(providers);
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
    sqlx::query("update provider_accounts set authentication_kind='oauth' where id='acct_right'")
        .execute(&db.pool)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let cleaned = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(ProbeProvider {
        kind: ProviderKind::new("openai").unwrap(),
        pending: true,
        calls: calls.clone(),
        cleaned: cleaned.clone(),
        preference_fixture: None,
        gate: None,
    });
    let providers = ProviderAdminRegistry::new([provider as Arc<dyn ProviderAdmin>]).unwrap();
    let task = task.with_turnstate(providers.clone());
    let service = service.with_turnstate(providers);
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
