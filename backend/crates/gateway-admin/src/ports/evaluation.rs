//! 测评持久化端口；Admin 决定执行策略，Store 保证队列与游标事务

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::model::{MutationContext, evaluation::*};
use crate::ports::store::AdminStoreResult;

#[async_trait]
pub trait EvaluationRepository: Send + Sync {
    async fn account_states(&self) -> AdminStoreResult<Vec<EvaluationAccountState>>;
    async fn reconcile_accounts(&self) -> AdminStoreResult<()>;
    async fn is_work_stop_requested(
        &self,
        item_id: &str,
        judgment_id: Option<&str>,
    ) -> AdminStoreResult<bool>;
    async fn settings(&self) -> AdminStoreResult<EvaluationSettings>;
    async fn update_settings(
        &self,
        command: UpdateEvaluationSettings,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationSettings>;
    async fn typesafe_api_key(&self) -> AdminStoreResult<Option<secrecy::SecretString>>;
    async fn claim_judgment(
        &self,
        now: DateTime<Utc>,
    ) -> AdminStoreResult<Option<ClaimedEvaluationJudgment>>;
    async fn record_judgment_request(&self, id: &str, request_id: &str) -> AdminStoreResult<bool>;
    async fn save_judgment_output(&self, id: &str, output: &str) -> AdminStoreResult<bool>;
    async fn finish_judgment(
        &self,
        id: &str,
        completion: EvaluationJudgmentCompletion,
    ) -> AdminStoreResult<bool>;
    async fn retry_judgment(
        &self,
        item_id: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<()>;
    /// 同一管理员与提交标识只创建一次；参数变化返回冲突，重放不新增审计
    async fn enqueue_manual(
        &self,
        submission_id: uuid::Uuid,
        seed: EvaluationRunSeed,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationRun>;

    /// 检查计划版本、游标与启用状态，推进游标并创建任务或跳过记录
    async fn enqueue_scheduled(
        &self,
        command: EnqueueScheduledEvaluation,
    ) -> AdminStoreResult<bool>;

    async fn list_runs(&self, limit: u32, offset: u32) -> AdminStoreResult<Vec<EvaluationRun>>;
    async fn history_sources(&self) -> AdminStoreResult<Vec<EvaluationHistorySource>>;
    async fn account_history(
        &self,
        account_ids: &[String],
        limit: u32,
        source_id: Option<&str>,
    ) -> AdminStoreResult<Vec<EvaluationHistoryItem>>;
    async fn load_run(&self, id: &str) -> AdminStoreResult<Option<EvaluationRunDetail>>;
    async fn stop_run(&self, id: &str, context: &MutationContext) -> AdminStoreResult<bool>;

    /// 原子领取；全局最多 2 个，且同账号最多 1 个运行任务
    async fn claim_next(&self, now: DateTime<Utc>) -> AdminStoreResult<Option<ClaimedEvaluation>>;
    async fn is_stop_requested(&self, run_id: &str) -> AdminStoreResult<bool>;
    async fn record_request_id(&self, item_id: &str, request_id: &str) -> AdminStoreResult<bool>;
    async fn save_output(&self, item_id: &str, output: EvaluationOutput) -> AdminStoreResult<bool>;
    async fn load_output(&self, item_id: &str) -> AdminStoreResult<Option<EvaluationOutput>>;
    async fn finish_item(
        &self,
        item_id: &str,
        completion: EvaluationCompletion,
    ) -> AdminStoreResult<bool>;

    /// 仅在 Worker 启动时调用；运行中请求一律中断，不能自动重放可能已计费的调用
    async fn recover_running(&self, now: DateTime<Utc>) -> AdminStoreResult<u64>;
    async fn prune_history(&self, policy: EvaluationRetention) -> AdminStoreResult<u64>;

    async fn list_plans(&self) -> AdminStoreResult<Vec<EvaluationPlan>>;
    async fn save_plan(
        &self,
        command: SaveEvaluationPlan,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationPlan>;
    async fn delete_plan(
        &self,
        id: &str,
        expected_revision: i64,
        context: &MutationContext,
    ) -> AdminStoreResult<bool>;
}

/// Host 适配官方专用接口，Admin 持有评分策略、取消与持久化
#[async_trait]
pub trait TypeSafeJudgeClient: Send + Sync {
    async fn judge(
        &self,
        api_key: &secrecy::SecretString,
        config: &TypeSafeJudgeConfig,
        definition: &EvaluationDefinition,
        response: &str,
    ) -> Result<String, &'static str>;
}
