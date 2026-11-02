//! 正式监测快照的领域投影，状态、筛选及优选不随临时诊断切换

use gateway_core::{
    policy::ClientApiKeyId,
    routing::{ProviderKind, RuntimeSnapshot},
    runtime::RuntimeSnapshotHandle,
};
use sha2::{Digest as _, Sha256};

use crate::{
    model::{AdminError, evaluation::*},
    ports::provider::ProviderAdminRegistry,
};

pub(super) fn profile_revision(
    snapshot: &RuntimeSnapshot,
    key: &str,
    providers: Option<&ProviderAdminRegistry>,
) -> Result<String, AdminError> {
    let kind =
        ProviderKind::new("openai").map_err(|_| AdminError::internal("测票 Provider 无效"))?;
    let key = ClientApiKeyId::new(key).map_err(|_| AdminError::invalid("测票 Key 无效"))?;
    let profile = snapshot
        .client_policy(&key)
        .and_then(|policy| policy.account_scope().request_profile(&kind))
        .or_else(|| snapshot.settings().request_profiles().get(&kind));
    let wire = providers
        .and_then(|providers| providers.require(&kind).ok())
        .and_then(|provider| match profile {
            Some(profile) => provider.configured_wire_profile(profile),
            None => provider.dashboard_wire_profile(),
        })
        .map(|profile| (profile.product, profile.version, profile.user_agent));
    let bytes = serde_json::to_vec(&(profile.map(|profile| profile.expose_to_provider()), wire))
        .map_err(|_| AdminError::internal("测票身份配置无法读取"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn project(
    data: EvaluationMonitoringData,
    snapshots: &RuntimeSnapshotHandle,
    providers: Option<&ProviderAdminRegistry>,
) -> Result<EvaluationMonitoringSnapshot, AdminError> {
    let (selected, revision, policy) = if let Some(plan) = &data.plan {
        let snapshot = snapshots
            .acquire()
            .map_err(|_| AdminError::internal("当前监测配置不可用"))?;
        (
            super::accounts::resolve(&plan.settings, &data.accounts, snapshots, providers)?,
            profile_revision(
                &snapshot,
                &plan.settings.parameters.client_key_id,
                providers,
            )?,
            plan.settings.monitoring.clone().unwrap_or_default(),
        )
    } else {
        (
            Vec::new(),
            String::new(),
            EvaluationMonitoringPolicy::default(),
        )
    };
    let selected = selected
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let version = data
        .plan
        .as_ref()
        .map(|plan| plan.monitoring_reference().version);
    let previous_accounts = data
        .points
        .iter()
        .map(|point| point.history.account_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let current_points = data
        .points
        .iter()
        .filter(|point| Some(&point.plan_version) == version.as_ref())
        .collect::<Vec<_>>();
    let mut grouped = std::collections::BTreeMap::<&str, Vec<&EvaluationMonitoringPoint>>::new();
    for point in &current_points {
        grouped
            .entry(&point.history.account_id)
            .or_default()
            .push(point);
    }
    let accounts = data
        .accounts
        .iter()
        .map(|account| {
            let mut state = EvaluationAccountMonitoring::derive(
                account,
                super::accounts::supports(account, EvaluationKind::Turnstate, providers),
                selected.contains(&account.id),
                &policy,
                grouped
                    .get(account.id.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                &revision,
                data.observed_at,
            );
            if state.monitored
                && state.freshness == EvaluationFreshness::Unmeasured
                && !grouped.contains_key(account.id.as_str())
                && previous_accounts.contains(&account.id)
            {
                state.status = EvaluationAccountStatus::Stale;
                state.freshness = EvaluationFreshness::EnvironmentChanged;
            }
            state
        })
        .collect();
    let mut history_counts = std::collections::BTreeMap::<String, usize>::new();
    Ok(EvaluationMonitoringSnapshot {
        observed_at: data.observed_at,
        plan: data.plan,
        accounts,
        history: data
            .points
            .into_iter()
            .filter(|point| Some(&point.plan_version) == version.as_ref())
            .map(|point| point.history)
            .filter(|point| {
                let count = history_counts.entry(point.account_id.clone()).or_default();
                *count += 1;
                *count <= 28
            })
            .collect(),
    })
}
