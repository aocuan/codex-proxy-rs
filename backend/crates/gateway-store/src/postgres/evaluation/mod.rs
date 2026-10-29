//! 测评记录的 PostgreSQL 事务适配；权限与实验策略由 Admin/Core 解释

mod accounts;
mod history;
mod items;
mod judgments;
mod plans;
mod retention;
mod rows;
mod runs;
mod settings;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gateway_admin::{
    model::{MutationContext, audit::MutationAuditOperation, evaluation::*},
    ports::{
        evaluation::EvaluationRepository,
        store::{AdminStoreError, AdminStoreErrorKind, AdminStoreResult},
    },
};
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Clone)]
pub struct PgEvaluationRepository {
    pool: PgPool,
}

impl PgEvaluationRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EvaluationRepository for PgEvaluationRepository {
    async fn account_states(&self) -> AdminStoreResult<Vec<EvaluationAccountState>> {
        accounts::list(&self.pool).await
    }
    async fn reconcile_accounts(&self) -> AdminStoreResult<()> {
        accounts::reconcile(&self.pool).await
    }
    async fn is_work_stop_requested(
        &self,
        item_id: &str,
        judgment_id: Option<&str>,
    ) -> AdminStoreResult<bool> {
        accounts::stopped(&self.pool, item_id, judgment_id).await
    }
    async fn settings(&self) -> AdminStoreResult<EvaluationSettings> {
        settings::load(&self.pool).await
    }
    async fn update_settings(
        &self,
        command: UpdateEvaluationSettings,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationSettings> {
        settings::save(&self.pool, command, context).await
    }
    async fn typesafe_api_key(&self) -> AdminStoreResult<Option<secrecy::SecretString>> {
        settings::key(&self.pool).await
    }
    async fn claim_judgment(
        &self,
        now: DateTime<Utc>,
    ) -> AdminStoreResult<Option<ClaimedEvaluationJudgment>> {
        judgments::claim(&self.pool, now).await
    }
    async fn record_judgment_request(&self, id: &str, request_id: &str) -> AdminStoreResult<bool> {
        judgments::record_request(&self.pool, id, request_id).await
    }
    async fn save_judgment_output(&self, id: &str, output: &str) -> AdminStoreResult<bool> {
        judgments::save_output(&self.pool, id, output).await
    }
    async fn finish_judgment(
        &self,
        id: &str,
        completion: EvaluationJudgmentCompletion,
    ) -> AdminStoreResult<bool> {
        judgments::finish(&self.pool, id, completion).await
    }
    async fn retry_judgment(
        &self,
        item_id: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<()> {
        judgments::retry(&self.pool, item_id, context).await
    }
    async fn enqueue_manual(
        &self,
        submission_id: uuid::Uuid,
        seed: EvaluationRunSeed,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationRun> {
        runs::enqueue_manual(&self.pool, submission_id, seed, context).await
    }

    async fn enqueue_scheduled(
        &self,
        command: EnqueueScheduledEvaluation,
    ) -> AdminStoreResult<bool> {
        runs::enqueue_scheduled(&self.pool, command).await
    }

    async fn list_runs(&self, limit: u32, offset: u32) -> AdminStoreResult<Vec<EvaluationRun>> {
        runs::list(&self.pool, limit, offset).await
    }

    async fn history_sources(&self) -> AdminStoreResult<Vec<EvaluationHistorySource>> {
        history::sources(&self.pool).await
    }

    async fn account_history(
        &self,
        account_ids: &[String],
        limit: u32,
        source_id: Option<&str>,
    ) -> AdminStoreResult<Vec<EvaluationHistoryItem>> {
        history::account_history(&self.pool, account_ids, limit, source_id).await
    }

    async fn load_run(&self, id: &str) -> AdminStoreResult<Option<EvaluationRunDetail>> {
        runs::load(&self.pool, id).await
    }

    async fn stop_run(&self, id: &str, context: &MutationContext) -> AdminStoreResult<bool> {
        runs::stop(&self.pool, id, context).await
    }

    async fn claim_next(&self, now: DateTime<Utc>) -> AdminStoreResult<Option<ClaimedEvaluation>> {
        items::claim(&self.pool, now).await
    }

    async fn is_stop_requested(&self, run_id: &str) -> AdminStoreResult<bool> {
        // 清理或删除不能让失去归属的运行任务继续调用
        sqlx::query_scalar("select stop_requested from evaluation_runs where id = $1")
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map(|value| value.unwrap_or(true))
            .map_err(db_error)
    }

    async fn record_request_id(&self, item_id: &str, request_id: &str) -> AdminStoreResult<bool> {
        let result = sqlx::query(
            "update evaluation_items set request_id = $2
             where id = $1 and status = 'running' and (request_id is null or request_id = $2)",
        )
        .bind(item_id)
        .bind(request_id)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn save_output(&self, item_id: &str, output: EvaluationOutput) -> AdminStoreResult<bool> {
        items::save_output(&self.pool, item_id, output).await
    }

    async fn load_output(&self, item_id: &str) -> AdminStoreResult<Option<EvaluationOutput>> {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "select response_text, artifact_html from evaluation_artifacts where item_id = $1",
        )
        .bind(item_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(row.map(|(response_text, artifact_html)| EvaluationOutput {
            response_text,
            artifact_html,
        }))
    }

    async fn finish_item(
        &self,
        item_id: &str,
        completion: EvaluationCompletion,
    ) -> AdminStoreResult<bool> {
        items::finish(&self.pool, item_id, completion).await
    }

    async fn recover_running(&self, now: DateTime<Utc>) -> AdminStoreResult<u64> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        lock_queue(&mut transaction).await?;
        let result = sqlx::query(
            "update evaluation_items
             set status = 'interrupted', verdict = 'ungraded', final_answer = null,
                 error_code = 'evaluation.restart_interrupted',
                 message = '服务重启，未确认完成的请求不会自动重试', completed_at = $1
             where status = 'running'",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
        let judgments = sqlx::query("update evaluation_judgments set status = 'interrupted',
            reason = '服务重启，未确认完成的判题不会自动重试', completed_at = $1 where status = 'running'")
            .bind(now).execute(&mut *transaction).await.map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        Ok(result.rows_affected() + judgments.rows_affected())
    }

    async fn list_plans(&self) -> AdminStoreResult<Vec<EvaluationPlan>> {
        plans::list(&self.pool).await
    }

    async fn prune_history(&self, policy: EvaluationRetention) -> AdminStoreResult<u64> {
        retention::prune(&self.pool, policy).await
    }

    async fn save_plan(
        &self,
        command: SaveEvaluationPlan,
        context: &MutationContext,
    ) -> AdminStoreResult<EvaluationPlan> {
        plans::save(&self.pool, command, context).await
    }

    async fn delete_plan(
        &self,
        id: &str,
        expected_revision: i64,
        context: &MutationContext,
    ) -> AdminStoreResult<bool> {
        plans::delete(&self.pool, id, expected_revision, context).await
    }
}

/// 队列事务很短；同一部署内串行核对容量与账号占用，不把模型调用放进锁
async fn lock_queue(transaction: &mut Transaction<'_, Postgres>) -> AdminStoreResult<()> {
    sqlx::query(
        "select pg_advisory_xact_lock(hashtextextended(current_schema() || ':evaluation_queue', 0))",
    )
    .execute(&mut **transaction)
    .await
    .map_err(db_error)?;
    Ok(())
}

async fn append_audit(
    transaction: &mut Transaction<'_, Postgres>,
    context: &MutationContext,
    operation: MutationAuditOperation,
    id: &str,
    fields: &[&str],
) -> AdminStoreResult<()> {
    let event = crate::mutation_audit(
        context,
        operation,
        id,
        fields.iter().map(|field| (*field).to_owned()).collect(),
    );
    super::admin_security_audit::append_admin_audit_event_in_transaction(transaction, event, None)
        .await
        .map_err(|source| {
            AdminStoreError::new(
                AdminStoreErrorKind::Unavailable,
                "evaluation",
                "测评审计写入失败",
            )
            .with_source(source)
        })
}

fn db_error(source: sqlx::Error) -> AdminStoreError {
    AdminStoreError::new(
        AdminStoreErrorKind::Unavailable,
        "evaluation",
        "测评存储暂时不可用",
    )
    .with_source(source)
}

fn conflict(message: &str) -> AdminStoreError {
    AdminStoreError::new(AdminStoreErrorKind::Conflict, "evaluation", message)
}

fn invalid(message: &str) -> AdminStoreError {
    AdminStoreError::new(AdminStoreErrorKind::Invalid, "evaluation", message)
}
