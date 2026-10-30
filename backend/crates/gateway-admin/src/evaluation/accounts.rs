//! 计划账号范围解析，账号事实来自仓储，Key 授权复用 Core 快照

use crate::model::{AdminError, evaluation::*};
use crate::ports::provider::ProviderAdminRegistry;
use gateway_core::{
    account::ProviderAccountId,
    policy::{ClientApiKeyId, ClientPolicy},
    runtime::RuntimeSnapshotHandle,
};

pub(super) fn independent_preference_key(policy: &ClientPolicy, group_id: &str) -> bool {
    !policy
        .account_scope()
        .routing_snapshot()
        .groups_snapshot()
        .iter()
        .any(|group| group.id().as_str() == group_id)
}

pub(super) fn supports(
    account: &EvaluationAccountState,
    kind: EvaluationKind,
    providers: Option<&ProviderAdminRegistry>,
) -> bool {
    if kind != EvaluationKind::Turnstate {
        return true;
    }
    supports_turnstate(
        &account.id,
        &account.provider_kind,
        &account.authentication_kind,
        providers,
    )
}

pub(super) fn supports_turnstate(
    account_id: &str,
    provider_kind: &str,
    authentication_kind: &str,
    providers: Option<&ProviderAdminRegistry>,
) -> bool {
    let (Some(providers), Ok(id), Ok(provider_kind)) = (
        providers,
        ProviderAccountId::new(account_id),
        gateway_core::routing::ProviderKind::new(provider_kind),
    ) else {
        return false;
    };
    providers.require(&provider_kind).is_ok_and(|provider| {
        provider
            .account_capabilities(&id, authentication_kind)
            .turn_state_probe
    })
}

pub(super) fn resolve(
    settings: &EvaluationPlanSettings,
    accounts: &[EvaluationAccountState],
    snapshots: &RuntimeSnapshotHandle,
    providers: Option<&ProviderAdminRegistry>,
) -> Result<Vec<String>, AdminError> {
    let snapshot = snapshots
        .acquire()
        .map_err(|_| AdminError::internal("当前账号授权配置不可用"))?;
    let key = ClientApiKeyId::new(settings.parameters.client_key_id.clone())
        .map_err(|_| AdminError::invalid("Client Key ID 无效"))?;
    let Some(policy) = snapshot
        .client_policy(&key)
        .filter(|policy| policy.authorize().is_ok())
    else {
        return Ok(Vec::new());
    };
    if settings
        .account_preference
        .as_ref()
        .is_some_and(|rule| !independent_preference_key(policy, &rule.group_id))
    {
        return Ok(Vec::new());
    }
    let judge = settings
        .parameters
        .definition
        .as_ref()
        .and_then(|d| match &d.grader {
            EvaluationGrader::Model { config } => Some(config.account_id.as_str()),
            _ => None,
        });
    Ok(accounts
        .iter()
        .filter(|account| {
            account.enabled
                && supports(account, settings.parameters.kind, providers)
                && judge != Some(account.id.as_str())
                && ProviderAccountId::new(account.id.clone())
                    .is_ok_and(|id| policy.account_scope().allows(&id))
                && match &settings.account_selection {
                    EvaluationAccountSelection::Fixed => {
                        settings.parameters.account_ids.contains(&account.id)
                    }
                    EvaluationAccountSelection::All => true,
                    EvaluationAccountSelection::Groups { group_ids } => {
                        account.group_ids.iter().any(|id| group_ids.contains(id))
                    }
                }
        })
        .map(|account| account.id.clone())
        .collect())
}
