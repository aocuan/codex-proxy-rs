//! 管理员测评命令校验与仓储协调

use std::{collections::BTreeSet, sync::Arc};

use chrono::Utc;
use gateway_core::{
    account::ProviderAccountId, policy::ClientApiKeyId, runtime::RuntimeSnapshotHandle,
    time::DeploymentTimeZone,
};
use uuid::Uuid;

use crate::{
    model::{AdminError, MutationContext, evaluation::*},
    ports::evaluation::EvaluationRepository,
    use_case::map_store_error,
};

pub struct EvaluationService {
    repository: Arc<dyn EvaluationRepository>,
    version: String,
    timezone: DeploymentTimeZone,
    snapshot: RuntimeSnapshotHandle,
}

impl EvaluationService {
    /// 仅预检当前 Key 的账号授权，执行时仍由 Core 重新鉴权和准入
    pub fn account_access(
        &self,
        client_key_id: &str,
        account_ids: &[String],
    ) -> Result<Vec<EvaluationAccountAccess>, AdminError> {
        let key_id =
            ClientApiKeyId::new(client_key_id).map_err(|_| AdminError::invalid("Key ID 无效"))?;
        if account_ids.is_empty() || account_ids.len() > 120 {
            return Err(AdminError::invalid("每次检查 1 到 120 个账号"));
        }
        let accounts = account_ids
            .iter()
            .map(|id| {
                ProviderAccountId::new(id.clone()).map_err(|_| AdminError::invalid("账号 ID 无效"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let snapshot = self
            .snapshot
            .acquire()
            .map_err(|_| AdminError::internal("当前账号授权配置不可用，请稍后重试"))?;
        let policy = snapshot.client_policy(&key_id);
        Ok(accounts
            .into_iter()
            .map(|account| {
                let reason = match policy {
                    None => Some("所选 Key 已不存在，请重新选择".to_owned()),
                    Some(policy) if policy.authorize().is_err() => {
                        Some("所选 Key 已停用，请更换 Key".to_owned())
                    }
                    Some(policy) if !policy.account_scope().allows(&account) => Some(format!(
                        "Key「{}」未授权此账号，请更换 Key 或检查账号分组",
                        policy.name_snapshot().unwrap_or(client_key_id),
                    )),
                    Some(_) => None,
                };
                EvaluationAccountAccess {
                    account_id: account.to_string(),
                    allowed: reason.is_none(),
                    reason,
                }
            })
            .collect())
    }

    fn validate_access(&self, key: &str, accounts: &[String]) -> Result<(), AdminError> {
        for access in self.account_access(key, accounts)? {
            if let Some(reason) = access.reason {
                return Err(AdminError::invalid(format!(
                    "{reason}（{}）",
                    access.account_id
                )));
            }
        }
        Ok(())
    }

    fn validate_parameter_access(
        &self,
        parameters: &EvaluationParameters,
    ) -> Result<(), AdminError> {
        self.validate_access(&parameters.client_key_id, &parameters.account_ids)?;
        if let Some(EvaluationDefinition {
            grader: EvaluationGrader::Model { config },
            ..
        }) = &parameters.definition
        {
            self.validate_access(
                &config.client_key_id,
                std::slice::from_ref(&config.account_id),
            )?;
        }
        Ok(())
    }

    pub async fn settings(&self) -> Result<EvaluationSettings, AdminError> {
        self.repository
            .settings()
            .await
            .map_err(|error| map_store_error(error, "evaluation.settings"))
    }

    pub async fn update_settings(
        &self,
        command: UpdateEvaluationSettings,
        context: &MutationContext,
    ) -> Result<EvaluationSettings, AdminError> {
        command.validate()?;
        if let Some(EvaluationJudgeDefaults::Model {
            client_key_id,
            account_id,
            ..
        }) = &command.defaults.judge
        {
            self.validate_access(client_key_id, std::slice::from_ref(account_id))?;
        }
        self.repository
            .update_settings(command, context)
            .await
            .map_err(|error| map_store_error(error, "evaluation.update_settings"))
    }

    async fn validate_judge_connection(
        &self,
        parameters: &EvaluationParameters,
    ) -> Result<(), AdminError> {
        if parameters.definition.as_ref().is_some_and(|definition| {
            matches!(definition.grader, EvaluationGrader::Typesafe { .. })
        }) && !self.settings().await?.typesafe_configured
        {
            return Err(AdminError::invalid("请先在测评设置中配置 TypeSafe API Key"));
        }
        Ok(())
    }
    pub async fn retry_judgment(
        &self,
        item_id: &str,
        context: &MutationContext,
    ) -> Result<(), AdminError> {
        self.repository
            .retry_judgment(item_id, context)
            .await
            .map_err(|error| map_store_error(error, "evaluation.retry_judgment"))
    }
    #[must_use]
    pub fn new(
        repository: Arc<dyn EvaluationRepository>,
        version: String,
        timezone: DeploymentTimeZone,
        snapshot: RuntimeSnapshotHandle,
    ) -> Self {
        Self {
            repository,
            version,
            timezone,
            snapshot,
        }
    }

    pub async fn submit(
        &self,
        submission_id: Uuid,
        parameters: EvaluationParameters,
        context: &MutationContext,
    ) -> Result<EvaluationRun, AdminError> {
        parameters.validate()?;
        self.validate_parameter_access(&parameters)?;
        self.validate_judge_connection(&parameters).await?;
        self.repository
            .enqueue_manual(
                submission_id,
                EvaluationRunSeed::new(parameters, &self.version, Utc::now()),
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "evaluation.submit"))
    }

    pub async fn list_runs(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<EvaluationRun>, AdminError> {
        if !(1..=100).contains(&limit) {
            return Err(AdminError::invalid("每页记录数必须在 1 到 100 之间"));
        }
        self.repository
            .list_runs(limit, offset)
            .await
            .map_err(|error| map_store_error(error, "evaluation.list_runs"))
    }

    pub async fn detail(&self, id: &str) -> Result<EvaluationRunDetail, AdminError> {
        self.repository
            .load_run(id)
            .await
            .map_err(|error| map_store_error(error, "evaluation.detail"))?
            .ok_or_else(|| AdminError::not_found("测评记录不存在或已清理"))
    }

    pub async fn history_sources(&self) -> Result<Vec<EvaluationHistorySource>, AdminError> {
        self.repository
            .history_sources()
            .await
            .map_err(|error| map_store_error(error, "evaluation.history_sources"))
    }

    pub async fn account_history(
        &self,
        account_ids: &[String],
        limit: u32,
        source_id: Option<&str>,
    ) -> Result<Vec<EvaluationHistoryItem>, AdminError> {
        if account_ids.is_empty() || account_ids.len() > 100 || !(1..=28).contains(&limit) {
            return Err(AdminError::invalid(
                "每次查询 1 到 100 个账号，每账号最多 28 条记录",
            ));
        }
        let mut unique = BTreeSet::new();
        for id in account_ids {
            ProviderAccountId::new(id.clone()).map_err(|_| AdminError::invalid("账号 ID 无效"))?;
            if !unique.insert(id) {
                return Err(AdminError::invalid("账号不能重复"));
            }
        }
        if source_id.is_some_and(|id| {
            id.len() != 32
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }) {
            return Err(AdminError::invalid("检测来源 ID 无效"));
        }
        self.repository
            .account_history(account_ids, limit, source_id)
            .await
            .map_err(|error| map_store_error(error, "evaluation.account_history"))
    }

    pub async fn stop(
        &self,
        id: &str,
        context: &MutationContext,
    ) -> Result<EvaluationRunDetail, AdminError> {
        self.detail(id).await?;
        self.repository
            .stop_run(id, context)
            .await
            .map_err(|error| map_store_error(error, "evaluation.stop"))?;
        self.detail(id).await
    }

    pub async fn output(&self, item_id: &str) -> Result<EvaluationOutput, AdminError> {
        self.repository
            .load_output(item_id)
            .await
            .map_err(|error| map_store_error(error, "evaluation.output"))?
            .ok_or_else(|| AdminError::not_found("测评输出尚未保存或已清理"))
    }

    pub async fn list_plans(&self) -> Result<Vec<EvaluationPlan>, AdminError> {
        let mut plans = self
            .repository
            .list_plans()
            .await
            .map_err(|error| map_store_error(error, "evaluation.list_plans"))?;
        let accounts = self
            .repository
            .account_states()
            .await
            .map_err(|error| map_store_error(error, "evaluation.accounts"))?;
        for plan in &mut plans {
            plan.available_accounts =
                Some(super::accounts::resolve(&plan.settings, &accounts, &self.snapshot)?.len());
        }
        Ok(plans)
    }

    pub async fn save_plan(
        &self,
        id: Option<String>,
        expected_revision: Option<i64>,
        mut settings: EvaluationPlanSettings,
        context: &MutationContext,
    ) -> Result<EvaluationPlan, AdminError> {
        if id.is_some() != expected_revision.is_some()
            || expected_revision.is_some_and(|value| value < 1)
        {
            return Err(AdminError::invalid("修改计划必须携带当前版本"));
        }
        // 时区由部署决定，不能由请求伪造显示为另一套调度时间
        settings.timezone = self.timezone;
        let next_run_at = settings.next_run_after(Utc::now())?;
        if settings.enabled {
            if settings.account_selection.is_fixed() {
                self.validate_parameter_access(&settings.parameters)?;
            } else {
                let snapshot = self
                    .snapshot
                    .acquire()
                    .map_err(|_| AdminError::internal("当前账号授权配置不可用"))?;
                let key = ClientApiKeyId::new(settings.parameters.client_key_id.clone())
                    .map_err(|_| AdminError::invalid("Client Key ID 无效"))?;
                if !snapshot
                    .client_policy(&key)
                    .is_some_and(|policy| policy.authorize().is_ok())
                {
                    return Err(AdminError::invalid("所选 Key 不存在或已停用"));
                }
                if let Some(EvaluationDefinition {
                    grader: EvaluationGrader::Model { config },
                    ..
                }) = &settings.parameters.definition
                {
                    self.validate_access(
                        &config.client_key_id,
                        std::slice::from_ref(&config.account_id),
                    )?;
                }
            }
            self.validate_judge_connection(&settings.parameters).await?;
        }
        self.repository
            .save_plan(
                SaveEvaluationPlan {
                    id: id.unwrap_or_else(|| format!("eval_plan_{}", Uuid::now_v7().simple())),
                    expected_revision,
                    settings,
                    next_run_at,
                },
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "evaluation.save_plan"))
    }

    pub async fn delete_plan(
        &self,
        id: &str,
        revision: i64,
        context: &MutationContext,
    ) -> Result<(), AdminError> {
        if revision < 1 {
            return Err(AdminError::invalid("计划版本无效"));
        }
        self.repository
            .delete_plan(id, revision, context)
            .await
            .map_err(|error| map_store_error(error, "evaluation.delete_plan"))?;
        Ok(())
    }
}
