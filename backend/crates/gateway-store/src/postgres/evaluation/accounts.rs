//! 账号生命周期事实与测评子任务停止标记，历史输出不随账号删除

use super::{db_error, lock_queue};
use gateway_admin::{model::evaluation::EvaluationAccountState, ports::store::AdminStoreResult};
use sqlx::PgPool;

#[derive(sqlx::FromRow)]
struct AccountStateRow {
    id: String,
    enabled: bool,
    group_ids: Vec<String>,
    provider_kind: String,
    authentication_kind: String,
    credential_revision: i64,
    proxy_url: Option<String>,
}

pub(super) async fn list(pool: &PgPool) -> AdminStoreResult<Vec<EvaluationAccountState>> {
    let mut connection = pool.acquire().await.map_err(db_error)?;
    list_on(&mut connection).await
}

pub(super) async fn list_on(
    connection: &mut sqlx::PgConnection,
) -> AdminStoreResult<Vec<EvaluationAccountState>> {
    let rows = sqlx::query_as::<_, AccountStateRow>(
        "select a.id, a.enabled, array(select g.account_group_id from account_group_accounts g where g.provider_account_id = a.id order by g.account_group_id) as group_ids, a.provider_kind, a.authentication_kind, a.credential_revision,
         case when coalesce(p.enabled, true) then a.outbound_proxy_url else null end as proxy_url
         from provider_accounts a left join outbound_proxies p on p.id=a.outbound_proxy_id order by a.id")
        .fetch_all(connection).await.map_err(db_error)?;
    rows.into_iter()
        .map(|row| {
            let AccountStateRow {
                id,
                enabled,
                group_ids,
                provider_kind,
                authentication_kind,
                credential_revision: revision,
                proxy_url: proxy,
            } = row;
            Ok(EvaluationAccountState {
                connection_revision: connection_revision(&id, revision, proxy.as_deref())?,
                id,
                enabled,
                group_ids,
                provider_kind,
                authentication_kind,
            })
        })
        .collect()
}

fn connection_revision(id: &str, revision: i64, proxy: Option<&str>) -> AdminStoreResult<String> {
    use gateway_core::account::{
        CredentialRevision, OutboundProxy, ProviderAccountId, UpstreamConnection,
    };
    let account_id = ProviderAccountId::new(id).map_err(|_| super::invalid("账号 ID 无效"))?;
    let revision = CredentialRevision::new(
        u64::try_from(revision).map_err(|_| super::invalid("凭据版本无效"))?,
    )
    .map_err(|_| super::invalid("凭据版本无效"))?;
    let proxy = proxy
        .map(OutboundProxy::parse)
        .transpose()
        .map_err(|_| super::invalid("账号代理无效"))?;
    Ok(UpstreamConnection::new(account_id, revision, proxy.as_ref()).revision)
}

pub(super) async fn current_connection(
    connection: &mut sqlx::PgConnection,
    id: &str,
) -> AdminStoreResult<Option<String>> {
    let value: Option<(i64, Option<String>)> = sqlx::query_as("select a.credential_revision, case when coalesce(p.enabled,true) then a.outbound_proxy_url else null end from provider_accounts a left join outbound_proxies p on p.id=a.outbound_proxy_id where a.id=$1")
        .bind(id).fetch_optional(connection).await.map_err(db_error)?;
    value
        .map(|(revision, proxy)| connection_revision(id, revision, proxy.as_deref()))
        .transpose()
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
