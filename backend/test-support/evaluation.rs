//! 测评测试端口允许显式配置历史查询，未配置的方法拒绝意外调用

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use gateway_admin::{
    model::{MutationContext, evaluation::*},
    ports::{evaluation::EvaluationRepository, store::AdminStoreResult},
};
use gateway_core::{engine::execution::*, error::GatewayError, routing::PublicModelId};

#[derive(Default)]
pub struct TestEvaluationRepository {
    pub account_results: Mutex<Option<AdminStoreResult<Vec<EvaluationAccountResultFact>>>>,
    pub history: Mutex<Option<AdminStoreResult<Vec<EvaluationHistoryItem>>>>,
    pub history_queries: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl EvaluationRepository for TestEvaluationRepository {
    async fn account_result_facts(&self) -> AdminStoreResult<Vec<EvaluationAccountResultFact>> {
        self.account_results
            .lock()
            .unwrap()
            .clone()
            .expect("this fixture does not exercise evaluation account filtering")
    }

    async fn account_states(&self) -> AdminStoreResult<Vec<EvaluationAccountState>> {
        unreachable!()
    }
    async fn reconcile_accounts(&self) -> AdminStoreResult<()> {
        unreachable!()
    }
    async fn is_work_stop_requested(&self, _: &str, _: Option<&str>) -> AdminStoreResult<bool> {
        unreachable!()
    }
    async fn settings(&self) -> AdminStoreResult<EvaluationSettings> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn update_settings(
        &self,
        _command: UpdateEvaluationSettings,
        _context: &MutationContext,
    ) -> AdminStoreResult<EvaluationSettings> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn typesafe_api_key(&self) -> AdminStoreResult<Option<secrecy::SecretString>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn claim_judgment(
        &self,
        _now: DateTime<Utc>,
    ) -> AdminStoreResult<Option<ClaimedEvaluationJudgment>> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn record_judgment_request(
        &self,
        _id: &str,
        _request_id: &str,
    ) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn save_judgment_output(&self, _id: &str, _output: &str) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn finish_judgment(
        &self,
        _id: &str,
        _completion: EvaluationJudgmentCompletion,
    ) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn retry_judgment(
        &self,
        _item_id: &str,
        _context: &MutationContext,
    ) -> AdminStoreResult<()> {
        unreachable!("this fixture does not exercise evaluations")
    }
    async fn enqueue_manual(
        &self,
        _submission_id: uuid::Uuid,
        _seed: EvaluationRunSeed,
        _context: &MutationContext,
    ) -> AdminStoreResult<EvaluationRun> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn enqueue_scheduled(
        &self,
        _command: EnqueueScheduledEvaluation,
    ) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn list_runs(&self, _limit: u32, _offset: u32) -> AdminStoreResult<Vec<EvaluationRun>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn history_sources(&self) -> AdminStoreResult<Vec<EvaluationHistorySource>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn account_history(
        &self,
        account_ids: &[String],
        _limit: u32,
        _source_id: Option<&str>,
    ) -> AdminStoreResult<Vec<EvaluationHistoryItem>> {
        self.history_queries
            .lock()
            .unwrap()
            .push(account_ids.to_vec());
        self.history
            .lock()
            .unwrap()
            .clone()
            .expect("this fixture does not exercise evaluation history")
    }

    async fn load_run(&self, _id: &str) -> AdminStoreResult<Option<EvaluationRunDetail>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn stop_run(&self, _id: &str, _context: &MutationContext) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn claim_next(&self, _now: DateTime<Utc>) -> AdminStoreResult<Option<ClaimedEvaluation>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn is_stop_requested(&self, _run_id: &str) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn record_request_id(&self, _item_id: &str, _request_id: &str) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn save_output(
        &self,
        _item_id: &str,
        _output: EvaluationOutput,
    ) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn load_output(&self, _item_id: &str) -> AdminStoreResult<Option<EvaluationOutput>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn finish_item(
        &self,
        _item_id: &str,
        _completion: EvaluationCompletion,
    ) -> AdminStoreResult<EvaluationFinish> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn recover_running(&self, _now: DateTime<Utc>) -> AdminStoreResult<u64> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn prune_history(&self, _policy: EvaluationRetention) -> AdminStoreResult<u64> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn list_plans(&self) -> AdminStoreResult<Vec<EvaluationPlan>> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn save_plan(
        &self,
        _command: SaveEvaluationPlan,
        _context: &MutationContext,
    ) -> AdminStoreResult<EvaluationPlan> {
        unreachable!("this fixture does not exercise evaluations")
    }

    async fn delete_plan(
        &self,
        _id: &str,
        _expected_revision: i64,
        _context: &MutationContext,
    ) -> AdminStoreResult<bool> {
        unreachable!("this fixture does not exercise evaluations")
    }
}

pub struct UnusedEvaluationExecution;

impl ExecutionService for UnusedEvaluationExecution {
    fn authenticate(&self, _: &str) -> Result<AuthenticatedClient, ClientAuthenticationError> {
        unreachable!("unused evaluation execution")
    }
    fn public_models(&self, _: &AuthenticatedClient) -> Vec<PublicModelId> {
        unreachable!("unused evaluation execution")
    }
    fn contains_public_model(&self, _: &AuthenticatedClient, _: &PublicModelId) -> bool {
        unreachable!("unused evaluation execution")
    }
    fn start(&self, _: StartExecution) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        unreachable!("unused evaluation execution")
    }
    fn start_provider_endpoint(
        &self,
        _: StartProviderExecution,
    ) -> BoxFuture<'_, Result<StartedExecution, GatewayError>> {
        unreachable!("unused evaluation execution")
    }
}
