//! 判题队列的原子领取、输出保存、重试与结论持久化

use chrono::{DateTime, Utc};
use gateway_admin::{
    model::{MutationContext, audit::MutationAuditOperation, evaluation::*},
    ports::store::AdminStoreResult,
};
use sqlx::{PgPool, Postgres, Transaction, types::Json};

use super::{append_audit, conflict, db_error, invalid, lock_queue};

pub(super) const VIEW: &str = "jsonb_build_object(
    'id', j.id, 'itemId', j.item_id, 'accountId', j.account_id, 'attempt', j.attempt,
    'status', j.status, 'requestId', j.request_id, 'verdict', j.verdict, 'reason', j.reason,
    'observation', j.observation_json, 'createdAt', j.created_at, 'completedAt', j.completed_at)";

pub(super) async fn enqueue(
    transaction: &mut Transaction<'_, Postgres>,
    item_id: &str,
    account_id: Option<&str>,
    attempt: i32,
) -> AdminStoreResult<()> {
    sqlx::query(
        "insert into evaluation_judgments (id, item_id, account_id, attempt, status, created_at)
        values ($1, $2, $3, $4, 'queued', now())",
    )
    .bind(format!("eval_judge_{}", uuid::Uuid::now_v7().simple()))
    .bind(item_id)
    .bind(account_id)
    .bind(attempt)
    .execute(&mut **transaction)
    .await
    .map_err(db_error)?;
    Ok(())
}

pub(super) async fn claim(
    pool: &PgPool,
    _now: DateTime<Utc>,
) -> AdminStoreResult<Option<ClaimedEvaluationJudgment>> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let busy: i64 = sqlx::query_scalar(
        "select
        (select count(*) from evaluation_items where status = 'running') +
        (select count(*) from evaluation_judgments where status = 'running')",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(db_error)?;
    if busy >= MAX_PARALLEL_REQUESTS {
        transaction.commit().await.map_err(db_error)?;
        return Ok(None);
    }
    let id: Option<String> = sqlx::query_scalar("select j.id from evaluation_judgments j
        join evaluation_items i on i.id = j.item_id join evaluation_runs r on r.id = i.run_id
        where j.status = 'queued' and not r.stop_requested
          and not exists (select 1 from evaluation_items busy where busy.status = 'running' and busy.account_id = j.account_id)
          and not exists (select 1 from evaluation_judgments busy where busy.status = 'running' and busy.account_id is not distinct from j.account_id)
        order by j.created_at, j.id limit 1 for update of j")
        .fetch_optional(&mut *transaction).await.map_err(db_error)?;
    let Some(id) = id else {
        transaction.commit().await.map_err(db_error)?;
        return Ok(None);
    };
    sqlx::query("update evaluation_judgments set status = 'running' where id = $1")
        .bind(&id)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    let mut query = sqlx::QueryBuilder::new("select ");
    query
        .push(VIEW)
        .push(
            ", i.run_id, r.profile_json, a.response_text
        from evaluation_judgments j join evaluation_items i on i.id = j.item_id
        join evaluation_runs r on r.id = i.run_id join evaluation_artifacts a on a.item_id = i.id
        where j.id = ",
        )
        .push_bind(id);
    let (judgment, run_id, profile, response_text): (
        Json<EvaluationJudgment>,
        String,
        Json<EvaluationProfile>,
        String,
    ) = query
        .build_query_as()
        .fetch_one(&mut *transaction)
        .await
        .map_err(db_error)?;
    let definition = profile
        .0
        .definition
        .ok_or_else(|| invalid("判题任务缺少测评定义"))?;
    transaction.commit().await.map_err(db_error)?;
    Ok(Some(ClaimedEvaluationJudgment {
        judgment: judgment.0,
        run_id,
        definition,
        response_text,
    }))
}

pub(super) async fn record_request(
    pool: &PgPool,
    id: &str,
    request_id: &str,
) -> AdminStoreResult<bool> {
    let result = sqlx::query(
        "update evaluation_judgments set request_id = $2
        where id = $1 and status = 'running' and (request_id is null or request_id = $2)",
    )
    .bind(id)
    .bind(request_id)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn save_output(pool: &PgPool, id: &str, output: &str) -> AdminStoreResult<bool> {
    if output.len() > 65_536 {
        return Err(invalid("判题输出过长"));
    }
    let result = sqlx::query(
        "update evaluation_judgments set response_text = $2
        where id = $1 and status = 'running' and (response_text is null or response_text = $2)",
    )
    .bind(id)
    .bind(output)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn finish(
    pool: &PgPool,
    id: &str,
    result: EvaluationJudgmentCompletion,
) -> AdminStoreResult<bool> {
    if !matches!(
        result.status,
        EvaluationItemStatus::Succeeded
            | EvaluationItemStatus::Failed
            | EvaluationItemStatus::Cancelled
            | EvaluationItemStatus::Interrupted
    ) || result.verdict == EvaluationVerdict::NotApplicable
        || (result.status != EvaluationItemStatus::Succeeded
            && result.verdict != EvaluationVerdict::Ungraded)
        || result.reason.len() > 4096
    {
        return Err(invalid("判题终态无效"));
    }
    let updated = sqlx::query(
        "update evaluation_judgments
        set status = $2, verdict = $3, reason = $4, observation_json = $5, completed_at = $6
        where id = $1 and status = 'running' and ($2 <> 'succeeded' or response_text is not null)",
    )
    .bind(id)
    .bind(result.status.as_str())
    .bind(result.verdict.as_str())
    .bind(result.reason)
    .bind(Json(result.observation))
    .bind(result.completed_at)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(updated.rows_affected() == 1)
}

pub(super) async fn retry(
    pool: &PgPool,
    item_id: &str,
    context: &MutationContext,
) -> AdminStoreResult<()> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let row: Option<(Option<String>, i32, String, bool)> = sqlx::query_as(
        "select j.account_id, j.attempt, j.status, r.stop_requested
        from evaluation_judgments j join evaluation_items i on i.id = j.item_id
        join evaluation_runs r on r.id = i.run_id
        where i.id = $1 and i.status = 'succeeded'
        order by j.attempt desc limit 1 for update of j",
    )
    .bind(item_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    let (account, attempt, status, stopped) =
        row.ok_or_else(|| invalid("此记录没有可重试的 AI 判题"))?;
    if stopped || attempt >= 5 || !matches!(status.as_str(), "failed" | "interrupted" | "cancelled")
    {
        return Err(conflict(
            "仅能重试未成功的判题，最多 5 次；已停止的批次不能重试",
        ));
    }
    if !super::runs::has_capacity(&mut transaction, 1).await? {
        return Err(conflict("判题队列已满"));
    }
    enqueue(&mut transaction, item_id, account.as_deref(), attempt + 1).await?;
    append_audit(
        &mut transaction,
        context,
        MutationAuditOperation::EvaluationJudgmentRetry,
        item_id,
        &["judgment_retry"],
    )
    .await?;
    transaction.commit().await.map_err(db_error)?;
    Ok(())
}
