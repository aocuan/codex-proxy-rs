//! 在同一数据库快照中读取正式方案、账号连接身份及轻量历史

use gateway_admin::{model::evaluation::*, ports::store::AdminStoreResult};
use sqlx::PgPool;

use super::{db_error, rows};

#[derive(sqlx::FromRow)]
struct PointRow {
    plan_version: String,
    diagnostic: Option<sqlx::types::Json<TurnStateDiagnostic>>,
    #[sqlx(flatten)]
    history: rows::HistoryRow,
    connection_revision: Option<String>,
    request_profile_revision: Option<String>,
}

pub(super) async fn load(pool: &PgPool) -> AdminStoreResult<EvaluationMonitoringData> {
    let mut tx = pool.begin().await.map_err(db_error)?;
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let observed_at = sqlx::query_scalar("select now()")
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let plan: Option<rows::PlanRow> = sqlx::query_as(
        "select id, revision, settings_json, next_run_at, created_at, updated_at from evaluation_plans
         where settings_json->'monitoring' is not null and settings_json->'monitoring' <> 'null'::jsonb
         order by id limit 1")
        .fetch_optional(&mut *tx).await.map_err(db_error)?;
    let plan: Option<EvaluationPlan> = plan.map(Into::into);
    let accounts = super::accounts::list_on(&mut tx).await?;
    let mut points = Vec::new();
    if let Some(plan) = &plan {
        let records = sqlx::query_as::<_, PointRow>(
            "select h.* from provider_accounts a cross join lateral (
                with candidate as materialized (select i.id, i.run_id, i.account_id, i.status, i.verdict, i.final_answer,
                       i.started_at, i.completed_at, r.created_at, r.parameters_json->>'kind' as kind,
                       r.profile_json#>>'{monitoring,version}' as plan_version, r.parameters_json->>'model' as model, r.parameters_json->>'reasoningEffort' as reasoning_effort,
                       '快速测票'::text as name, r.profile_json->>'promptVersion' as profile_version,
                       'none'::text as grader_type, null::text as judgment_status, i.duration_ms,
                       i.observation_json #>> '{turnState,outcome}' as turn_state,
                       i.observation_json #>> '{turnState,connectionRevision}' as connection_revision,
                       i.observation_json->>'requestProfileRevision' as request_profile_revision,
                       nullif(i.observation_json#>'{turnState,diagnostic}', 'null'::jsonb) as diagnostic
                from evaluation_items i join evaluation_runs r on r.id=i.run_id
                where i.account_id=a.id and r.parameters_json->>'kind'='turnstate'
                  and r.profile_json#>>'{monitoring,id}' = $1), picked as (
                    (select id from candidate order by (status in ('queued','running')) desc, coalesce(started_at,created_at) desc, run_id desc, id desc limit 28)
                    union
                    (select id from candidate where plan_version=$2 and status not in ('queued','running','skipped','cancelled')
                     order by coalesce(started_at,created_at) desc, run_id desc, id desc limit 10)
                    union
                    (select id from candidate where plan_version=$2 and status='succeeded' and turn_state in ('accepted','refreshed')
                     order by coalesce(started_at,created_at) desc, run_id desc, id desc limit 1)
                    union
                    (select id from candidate where status='succeeded' and turn_state in ('accepted','refreshed')
                     order by coalesce(started_at,created_at) desc, run_id desc, id desc limit 1)
                ) select candidate.* from candidate join picked using(id)
             ) h order by h.account_id, (h.status in ('queued','running')) desc, coalesce(h.started_at,h.created_at) desc, h.run_id desc, h.id desc")
            .bind(&plan.id).bind(plan.monitoring_reference().version)
            .fetch_all(&mut *tx).await.map_err(db_error)?;
        for point in records {
            points.push(EvaluationMonitoringPoint {
                plan_version: point.plan_version,
                diagnostic: point.diagnostic.map(|value| value.0),
                history: point.history.decode()?,
                connection_revision: point.connection_revision,
                request_profile_revision: point.request_profile_revision,
            });
        }
    }
    tx.commit().await.map_err(db_error)?;
    Ok(EvaluationMonitoringData {
        observed_at,
        plan,
        accounts,
        points,
    })
}

pub(super) async fn cleanup(
    pool: &PgPool,
    command: EvaluationPreferenceCleanup,
) -> AdminStoreResult<Option<gateway_admin::model::Revision>> {
    let mut tx = pool.begin().await.map_err(db_error)?;
    super::lock_queue(&mut tx).await?;
    let plan: Option<rows::PlanRow> = sqlx::query_as("select id, revision, settings_json, next_run_at, created_at, updated_at from evaluation_plans where id=$1 and revision=$2 for update")
        .bind(&command.plan.id).bind(command.plan.revision).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let Some(plan) = plan.map(EvaluationPlan::from) else {
        return Ok(None);
    };
    if !plan.settings.enabled || plan.monitoring_reference() != command.plan {
        return Ok(None);
    }
    let (Some(rule), Some(policy)) = (&plan.settings.account_preference, &plan.settings.monitoring)
    else {
        return Ok(None);
    };
    let revision: i64 =
        sqlx::query_scalar("select config_revision from runtime_settings where id=1 for update")
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    if u64::try_from(revision).ok() != Some(command.expected_revision.get()) {
        return Ok(None);
    }
    if !super::preference::groups_enabled(
        &mut tx,
        &rule.group_id,
        rule.fallback_group_id.as_deref(),
    )
    .await?
    {
        return Ok(None);
    }
    let mut committed = None;
    for target in command.accounts {
        let id = &target.account_id;
        let Some((available, groups)) =
            super::accounts::lock_preference_account(&mut tx, id).await?
        else {
            continue;
        };
        if !groups.contains(&rule.group_id) || !plan.settings.includes_account(id, &groups) {
            continue;
        }
        let source = match target.reason {
            EvaluationPreferenceCleanupReason::AccountUnavailable if !available => {
                // 凭据失败仍需正式检测连续确认；后台仅处理明确停用的账号
                let enabled: bool =
                    sqlx::query_scalar("select enabled from provider_accounts where id=$1")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(db_error)?;
                if enabled {
                    continue;
                }
                "unavailable"
            }
            EvaluationPreferenceCleanupReason::Expired { last_signal_id }
                if policy.remove_expired =>
            {
                // 队列锁保证核对与移出之间没有新的正式结果提交
                let current_signal: Option<String> = sqlx::query_scalar(
                    "select i.id from evaluation_items i join evaluation_runs r on r.id=i.run_id
                    where i.account_id=$1 and r.profile_json#>>'{monitoring,id}'=$2 and i.status='succeeded'
                      and i.observation_json#>>'{turnState,outcome}' in ('accepted','refreshed')
                    order by coalesce(i.started_at,r.created_at) desc,r.id desc,i.id desc limit 1",
                )
                .bind(id)
                .bind(&plan.id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?;
                if current_signal != last_signal_id {
                    continue;
                }
                let connection = super::accounts::current_connection(&mut tx, id).await?;
                let signal_connection: Option<String> = sqlx::query_scalar("select observation_json#>>'{turnState,connectionRevision}' from evaluation_items where id=$1")
                    .bind(current_signal.as_deref()).fetch_optional(&mut *tx).await.map_err(db_error)?.flatten();
                // 新授权和代理变更后的旧信号不能撤销初始优选资格
                if connection.is_none() || signal_connection != connection {
                    continue;
                }
                "expiration"
            }
            _ => continue,
        };
        let request_id = format!("evaluation_{source}_{}", uuid::Uuid::new_v4().simple());
        if let Some(revision) = super::preference::write_memberships(
            &mut tx,
            id,
            &rule.group_id,
            rule.fallback_group_id.as_deref(),
            false,
            &request_id,
        )
        .await?
        {
            committed = Some(revision);
        }
    }
    tx.commit().await.map_err(db_error)?;
    Ok(committed)
}
