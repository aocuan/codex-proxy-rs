//! 管理员测票按显式范围选账号，能力测评继续复用 Core 的计费 Key 授权

use crate::model::{AdminError, evaluation::*};
use crate::ports::provider::ProviderAdminRegistry;
use gateway_core::{
    account::ProviderAccountId, policy::ClientApiKeyId, runtime::RuntimeSnapshotHandle,
};

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
    let snapshot = if settings.parameters.kind == EvaluationKind::Turnstate {
        None
    } else {
        Some(
            snapshots
                .acquire()
                .map_err(|_| AdminError::internal("当前账号授权配置不可用"))?,
        )
    };
    let policy = if let Some(snapshot) = &snapshot {
        let key = ClientApiKeyId::new(settings.parameters.client_key_id.clone())
            .map_err(|_| AdminError::invalid("Client Key ID 无效"))?;
        let Some(policy) = snapshot
            .client_policy(&key)
            .filter(|policy| policy.authorize().is_ok())
        else {
            return Ok(Vec::new());
        };
        Some(policy)
    } else {
        None
    };
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
                && policy.is_none_or(|policy| {
                    ProviderAccountId::new(account.id.clone())
                        .is_ok_and(|id| policy.account_scope().allows(&id))
                })
                && settings.includes_account(&account.id, &account.group_ids)
        })
        .map(|account| account.id.clone())
        .collect())
}
