//! 数据库列到测评领域记录的解码，不加载作品正文到列表

use chrono::{DateTime, Utc};
use gateway_admin::{model::evaluation::*, ports::store::AdminStoreResult};
use sqlx::{Postgres, QueryBuilder, types::Json};

pub(super) fn run_query() -> QueryBuilder<Postgres> {
    QueryBuilder::new(
        "select r.id, r.parameters_json, r.profile_json, r.plan_id, r.scheduled_at,
            r.stop_requested, r.created_at,
            (select count(*) from evaluation_items i where i.run_id = r.id) as retained_items,
            (select count(*) from evaluation_items i where i.run_id = r.id
                and (i.status in ('queued', 'running') or exists (select 1 from evaluation_judgments j where j.item_id = i.id and j.status in ('queued', 'running')))) as active_items,
            (select count(*) from evaluation_items i where i.run_id = r.id
                and i.status = 'succeeded') as succeeded_items,
            (select count(*) from evaluation_items i where i.run_id = r.id
                and coalesce((select j.verdict from evaluation_judgments j where j.item_id = i.id order by j.attempt desc limit 1), i.verdict) = 'correct') as correct_items
         from evaluation_runs r ",
    )
}

#[derive(sqlx::FromRow)]
pub(super) struct RunRow {
    id: String,
    parameters_json: Json<EvaluationParameters>,
    profile_json: Json<EvaluationProfile>,
    plan_id: Option<String>,
    scheduled_at: Option<DateTime<Utc>>,
    stop_requested: bool,
    created_at: DateTime<Utc>,
    retained_items: i64,
    active_items: i64,
    succeeded_items: i64,
    correct_items: i64,
}

impl From<RunRow> for EvaluationRun {
    fn from(row: RunRow) -> Self {
        Self {
            id: row.id,
            total_items: row.parameters_json.0.item_count() as i64,
            parameters: row.parameters_json.0,
            profile: row.profile_json.0,
            plan_id: row.plan_id,
            scheduled_at: row.scheduled_at,
            stop_requested: row.stop_requested,
            created_at: row.created_at,
            retained_items: row.retained_items,
            active_items: row.active_items,
            succeeded_items: row.succeeded_items,
            correct_items: row.correct_items,
        }
    }
}

pub(super) fn item_query() -> QueryBuilder<Postgres> {
    QueryBuilder::new(format!(
        "select i.id, i.run_id, i.account_id, i.repetition, i.status, i.request_id, coalesce(j.verdict, i.verdict) as verdict,
            i.final_answer, i.error_code, coalesce(j.reason, i.message) as message, i.ttft_ms, i.duration_ms,
            i.started_at, i.completed_at, i.observation_json,
            case when j.id is null then null else {} end as judgment,
            exists (select 1 from evaluation_artifacts a where a.item_id = i.id) as has_output,
            exists (select 1 from evaluation_artifacts a where a.item_id = i.id
                and a.artifact_html is not null) as has_artifact
         from evaluation_items i left join lateral
         (select * from evaluation_judgments latest where latest.item_id = i.id order by latest.attempt desc limit 1) j on true ",
        super::judgments::VIEW,
    ))
}

#[derive(sqlx::FromRow)]
pub(super) struct ItemRow {
    judgment: Option<Json<EvaluationJudgment>>,
    id: String,
    run_id: String,
    account_id: String,
    repetition: i32,
    status: String,
    request_id: Option<String>,
    verdict: String,
    final_answer: Option<i64>,
    error_code: Option<String>,
    message: Option<String>,
    ttft_ms: Option<i64>,
    duration_ms: Option<i64>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    has_output: bool,
    has_artifact: bool,
    observation_json: Json<EvaluationObservation>,
}

impl ItemRow {
    pub(super) fn decode(self) -> AdminStoreResult<EvaluationItem> {
        Ok(EvaluationItem {
            judgment: self.judgment.map(|value| value.0),
            id: self.id,
            run_id: self.run_id,
            account_id: self.account_id,
            repetition: self.repetition,
            status: decode_enum(self.status)?,
            request_id: self.request_id,
            verdict: decode_enum(self.verdict)?,
            final_answer: self.final_answer,
            error_code: self.error_code,
            message: self.message,
            ttft_ms: self.ttft_ms,
            duration_ms: self.duration_ms,
            started_at: self.started_at,
            completed_at: self.completed_at,
            has_output: self.has_output,
            has_artifact: self.has_artifact,
            observation: self.observation_json.0,
        })
    }
}

fn decode_enum<T: serde::de::DeserializeOwned>(value: String) -> AdminStoreResult<T> {
    serde_json::from_value(serde_json::Value::String(value))
        .map_err(|source| super::invalid("测评状态无效").with_source(source))
}

#[derive(sqlx::FromRow)]
pub(super) struct HistoryRow {
    name: String,
    profile_version: String,
    grader_type: String,
    judgment_status: Option<String>,
    id: String,
    run_id: String,
    account_id: String,
    kind: String,
    model: String,
    reasoning_effort: Option<String>,
    status: String,
    verdict: String,
    final_answer: Option<i64>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
}

impl HistoryRow {
    pub(super) fn decode(self) -> AdminStoreResult<EvaluationHistoryItem> {
        Ok(EvaluationHistoryItem {
            name: self.name,
            profile_version: self.profile_version,
            grader_type: self.grader_type,
            judgment_status: self.judgment_status.map(decode_enum).transpose()?,
            id: self.id,
            run_id: self.run_id,
            account_id: self.account_id,
            kind: decode_enum(self.kind)?,
            model: gateway_core::routing::PublicModelId::new(self.model)
                .map_err(|source| super::invalid("测评模型无效").with_source(source))?,
            reasoning_effort: self.reasoning_effort,
            status: decode_enum(self.status)?,
            verdict: decode_enum(self.verdict)?,
            final_answer: self.final_answer,
            created_at: self.created_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct PlanRow {
    id: String,
    revision: i64,
    settings_json: Json<EvaluationPlanSettings>,
    next_run_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PlanRow> for EvaluationPlan {
    fn from(row: PlanRow) -> Self {
        Self {
            id: row.id,
            revision: row.revision,
            settings: row.settings_json.0,
            next_run_at: row.next_run_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
