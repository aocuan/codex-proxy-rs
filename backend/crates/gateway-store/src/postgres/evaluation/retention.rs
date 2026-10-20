//! 按 Admin 提供的时间、数量和正文预算清理终态条目，活跃请求始终保留

use gateway_admin::{model::evaluation::EvaluationRetention, ports::store::AdminStoreResult};
use sqlx::PgPool;

use super::{db_error, invalid};

pub(super) async fn prune(pool: &PgPool, policy: EvaluationRetention) -> AdminStoreResult<u64> {
    if policy.keep_per_account_series < 1 || policy.max_output_bytes < 1 || policy.batch_size == 0 {
        return Err(invalid("测评保留阈值必须大于零"));
    }
    let mut transaction = pool.begin().await.map_err(db_error)?;
    super::lock_queue(&mut transaction).await?;
    let result = sqlx::query(
        "with terminal_ranks as (
            select i.id, row_number() over (
                partition by i.account_id, r.profile_json->>'promptVersion', r.parameters_json->>'model', r.parameters_json->>'reasoningEffort'
                order by i.completed_at desc, i.id desc
            ) as position
            from evaluation_items i join evaluation_runs r on r.id = i.run_id
            where i.status not in ('queued', 'running') and not exists (select 1 from evaluation_judgments j where j.item_id = i.id and j.status in ('queued', 'running'))
         ), body_ranks as (
            select i.id, sum(octet_length(a.response_text) + coalesce(octet_length(a.artifact_html), 0) + coalesce((select sum(octet_length(j.response_text)) from evaluation_judgments j where j.item_id = i.id), 0))
                over (order by i.completed_at desc nulls first, i.id desc) as accumulated_bytes
            from evaluation_artifacts a join evaluation_items i on i.id = a.item_id
         ), candidates as (
            select i.id from evaluation_items i
            join terminal_ranks t on t.id = i.id
            left join body_ranks b on b.id = i.id
            where i.completed_at < $1 or t.position > $2 or b.accumulated_bytes > $3
            order by i.completed_at, i.id limit $4
         )
         delete from evaluation_items where id in (select id from candidates)",
    )
    .bind(policy.completed_before)
    .bind(policy.keep_per_account_series)
    .bind(policy.max_output_bytes)
    .bind(i64::from(policy.batch_size.min(1000)))
    .execute(&mut *transaction)
    .await
    .map_err(db_error)?;
    // 空批次连同幂等标识一起过期，仍有保留条目的批次保存原始参数快照
    sqlx::query(
        "delete from evaluation_runs r where not exists
         (select 1 from evaluation_items i where i.run_id = r.id)",
    )
    .execute(&mut *transaction)
    .await
    .map_err(db_error)?;
    transaction.commit().await.map_err(db_error)?;
    Ok(result.rows_affected())
}
