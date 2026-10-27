//! 账号生命周期事实与测评子任务停止标记，历史输出不随账号删除

use super::{db_error, lock_queue};
use gateway_admin::{model::evaluation::EvaluationAccountState, ports::store::AdminStoreResult};
use sqlx::PgPool;

pub(super) async fn list(pool: &PgPool) -> AdminStoreResult<Vec<EvaluationAccountState>> {
    let rows: Vec<(String, bool, Vec<String>)> = sqlx::query_as(
        "select a.id, a.enabled, array(select g.account_group_id from account_group_accounts g where g.provider_account_id = a.id order by g.account_group_id)
         from provider_accounts a order by a.id")
        .fetch_all(pool).await.map_err(db_error)?;
    Ok(rows
        .into_iter()
        .map(|(id, enabled, group_ids)| EvaluationAccountState {
            id,
            enabled,
            group_ids,
        })
        .collect())
}

pub(super) async fn reconcile(pool: &PgPool) -> AdminStoreResult<()> {
    let mut tx = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut tx).await?;
    // 标记一经持久化不会因随后重新启用账号而复活，运行项由持有会话的 Worker 收尾
    sqlx::query("with active as (
          select id from evaluation_items where status in ('queued','running')
          union select item_id from evaluation_judgments where status in ('queued','running')
        ) update evaluation_items i set stop_requested = true
        from evaluation_runs r, active where r.id = i.run_id and active.id=i.id and not i.stop_requested
        and not exists (select 1 from provider_accounts a where a.id=i.account_id and a.enabled
          and (coalesce(r.profile_json->'accountSelection'->>'mode','fixed') <> 'groups'
            or exists (select 1 from account_group_accounts g where g.provider_account_id=a.id
              and (r.profile_json->'accountSelection'->'groupIds') ? g.account_group_id)))")
        .execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("update evaluation_judgments j set stop_requested=true
        from evaluation_items i where i.id=j.item_id and not j.stop_requested and j.status in ('queued','running')
        and (i.stop_requested or (j.account_id is not null and not exists (select 1 from provider_accounts a where a.id=j.account_id and a.enabled)))")
        .execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("update evaluation_items set status='skipped', completed_at=now(), error_code='evaluation.account_unavailable', message='账号已删除、停用或移出计划范围'
        where stop_requested and status='queued'")
        .execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("update evaluation_judgments set status='cancelled', completed_at=now(), reason='被测或判题账号已删除、停用或移出计划范围'
        where stop_requested and status='queued'")
        .execute(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)
}

pub(super) async fn stopped(
    pool: &PgPool,
    item: &str,
    judgment: Option<&str>,
) -> AdminStoreResult<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "select r.stop_requested or i.stop_requested or coalesce(j.stop_requested,false)
       from evaluation_items i join evaluation_runs r on r.id=i.run_id
       left join evaluation_judgments j on j.item_id=i.id and j.id=$2 where i.id=$1",
    )
    .bind(item)
    .bind(judgment)
    .fetch_optional(pool)
    .await
    .map_err(db_error)?
    .unwrap_or(true))
}
