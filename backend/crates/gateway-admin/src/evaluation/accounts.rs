//! 计划账号范围解析，账号事实来自仓储，Key 授权复用 Core 快照

use crate::model::{AdminError, evaluation::*};
use gateway_core::{
    account::ProviderAccountId, policy::ClientApiKeyId, runtime::RuntimeSnapshotHandle,
};

pub(super) fn resolve(
    settings: &EvaluationPlanSettings,
    accounts: &[EvaluationAccountState],
    snapshots: &RuntimeSnapshotHandle,
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
