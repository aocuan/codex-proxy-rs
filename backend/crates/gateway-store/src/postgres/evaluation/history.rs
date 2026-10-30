//! 账号检测来源与同条件历史查询，不加载回答正文或完整测票步骤

use gateway_admin::{model::evaluation::*, ports::store::AdminStoreResult};
use sqlx::{PgPool, Postgres, QueryBuilder};

use super::{db_error, rows};

// 来源标识只由实验条件生成，改名或新增同条件批次不会让已保存的选择失效
const SERIES: &str = "select r.id as run_id, r.created_at,
    md5(jsonb_build_array(r.parameters_json->>'kind', r.profile_json->>'promptVersion',
        r.profile_json->>'graderVersion', r.parameters_json->>'model',
        r.parameters_json->>'reasoningEffort')::text) as source_id,
    r.parameters_json->>'kind' as kind,
    r.parameters_json->>'model' as model,
    r.parameters_json->>'reasoningEffort' as reasoning_effort,
    coalesce(r.profile_json #>> '{definition,name}',
        case r.parameters_json->>'kind' when 'turnstate' then '快速测票'
            when 'pelican' then '画鹈鹕' else '糖果题' end) as name,
    r.profile_json->>'promptVersion' as profile_version,
    r.profile_json->>'graderVersion' as grader_version,
    coalesce(r.profile_json #>> '{definition,grader,type}',
        case when r.parameters_json->>'kind' = 'candy' then 'exact' else 'none' end) as grader_type
    from evaluation_runs r
    where (r.parameters_json->>'kind' = 'turnstate' or
        coalesce(r.profile_json #>> '{definition,grader,type}',
            case when r.parameters_json->>'kind' = 'candy' then 'exact' else 'none' end) <> 'none')
      and exists (select 1 from evaluation_items i where i.run_id = r.id)";

pub(super) async fn sources(pool: &PgPool) -> AdminStoreResult<Vec<EvaluationHistorySource>> {
    let mut query = QueryBuilder::<Postgres>::new("with series as (");
    query.push(SERIES).push(
        ")
         select source_id as id, name, kind, model, reasoning_effort,
                profile_version, grader_version, grader_type
         from (select distinct on (source_id) * from series
               order by source_id, created_at desc, run_id desc) latest
         order by created_at desc, run_id desc",
    );
    let records = query
        .build_query_as::<rows::HistorySourceRow>()
        .fetch_all(pool)
        .await
        .map_err(db_error)?;
    records
        .into_iter()
        .map(rows::HistorySourceRow::decode)
        .collect()
}

pub(super) async fn account_history(
    pool: &PgPool,
    account_ids: &[String],
    limit: u32,
    source_id: Option<&str>,
) -> AdminStoreResult<Vec<EvaluationHistoryItem>> {
    // 默认选全局最近的测票条件，各账号必须匹配同一个来源，不回退到其他题目或模型
    let mut query = QueryBuilder::<Postgres>::new("with series as (");
    query.push(SERIES).push(
        "), selected as (
            select source_id from series
            where ("
    ).push_bind(source_id).push("::text is null and kind = 'turnstate') or source_id = ")
    .push_bind(source_id).push("
            order by created_at desc, run_id desc limit 1
         )
         select h.* from unnest(").push_bind(account_ids).push("::text[]) as accounts(id)
         cross join lateral (
            select i.id, i.run_id, i.account_id, i.status, coalesce(j.verdict, i.verdict) as verdict,
                   i.final_answer, i.started_at, i.completed_at, s.created_at,
                   s.kind, s.model, s.reasoning_effort, s.name, s.profile_version, s.grader_type,
                   j.status as judgment_status, i.duration_ms,
                   i.observation_json #>> '{turnState,outcome}' as turn_state
            from evaluation_items i join series s on s.run_id = i.run_id
            join selected on selected.source_id = s.source_id
            left join lateral (select verdict, status from evaluation_judgments latest
                where latest.item_id = i.id order by latest.attempt desc limit 1) j on true
            where i.account_id = accounts.id
            order by s.created_at desc, s.run_id desc, i.repetition desc limit ")
    .push_bind(i64::from(limit)).push("
         ) h order by h.account_id, h.created_at desc, h.run_id desc, h.id desc"
    );
    let records = query
        .build_query_as::<rows::HistoryRow>()
        .fetch_all(pool)
        .await
        .map_err(db_error)?;
    records.into_iter().map(rows::HistoryRow::decode).collect()
}

pub(super) async fn account_result_facts(
    pool: &PgPool,
) -> AdminStoreResult<Vec<EvaluationAccountResultFact>> {
    let mut query = QueryBuilder::<Postgres>::new("with series as (");
    query.push(SERIES).push(
        "), selected as (
            select source_id from series where kind = 'turnstate'
            order by created_at desc, run_id desc limit 1
         )
         select a.id as account_id, a.provider_kind, a.authentication_kind,
                current.status, current.outcome
         from provider_accounts a
         left join lateral (
            select point.status, point.outcome from (
                select i.id, i.status, i.observation_json #>> '{turnState,outcome}' as outcome,
                       s.created_at, s.run_id
                from evaluation_items i join series s on s.run_id = i.run_id
                join selected on selected.source_id = s.source_id
                where i.account_id = a.id
                order by s.created_at desc, s.run_id desc, i.repetition desc limit 28
            ) point
            order by (point.status in ('queued','running')), point.created_at desc,
                     point.run_id desc, point.id desc limit 1
         ) current on true
         order by a.id",
    );
    let records = query
        .build_query_as::<rows::AccountResultRow>()
        .fetch_all(pool)
        .await
        .map_err(db_error)?;
    records
        .into_iter()
        .map(rows::AccountResultRow::decode)
        .collect()
}
