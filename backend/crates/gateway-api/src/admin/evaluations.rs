//! 模型测评管理路由；仅接受管理员身份，业务与队列生命周期由 Admin 持有

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use gateway_admin::model::evaluation::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    AdminAuth, AdminEnvelope, AdminError, AdminJson, AdminQuery, AdminResponse,
    wire::map_admin_service_error,
};
use crate::{auth::SessionState, time::TimePresenter};

pub(super) fn router<S>() -> Router<S>
where
    S: SessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/admin/evaluations/config", get(config::<S>))
        .route(
            "/api/admin/evaluations/accounts/access",
            post(account_access::<S>),
        )
        .route("/api/admin/evaluations/settings", get(settings::<S>))
        .route(
            "/api/admin/evaluations/settings/update",
            post(update_settings::<S>),
        )
        .route("/api/admin/evaluations/runs", get(list_runs::<S>))
        .route(
            "/api/admin/evaluations/accounts/history",
            get(account_history::<S>),
        )
        .route("/api/admin/evaluations/runs/create", post(submit::<S>))
        .route("/api/admin/evaluations/runs/detail", get(detail::<S>))
        .route("/api/admin/evaluations/runs/stop", post(stop::<S>))
        .route("/api/admin/evaluations/items/output", get(output::<S>))
        .route(
            "/api/admin/evaluations/items/retry-judgment",
            post(retry_judgment::<S>),
        )
        .route("/api/admin/evaluations/plans", get(list_plans::<S>))
        .route(
            "/api/admin/evaluations/plans/create",
            post(create_plan::<S>),
        )
        .route(
            "/api/admin/evaluations/plans/update",
            post(update_plan::<S>),
        )
        .route(
            "/api/admin/evaluations/plans/delete",
            post(delete_plan::<S>),
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountAccessRequest {
    client_key_id: String,
    account_ids: Vec<String>,
}

async fn account_access<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<AccountAccessRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let access = state
        .admin_services()
        .evaluations()
        .account_access(&request.client_key_id, &request.account_ids)
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(access),
    ))
}

async fn settings<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> Result<impl IntoResponse, AdminError> {
    let settings = state
        .admin_services()
        .evaluations()
        .settings()
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(settings),
    ))
}

async fn update_settings<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateEvaluationSettings>,
) -> Result<impl IntoResponse, AdminError> {
    let settings = state
        .admin_services()
        .evaluations()
        .update_settings(request, &auth.context().mutation_context())
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(settings),
    ))
}

async fn retry_judgment<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<IdRequest>,
) -> Result<impl IntoResponse, AdminError> {
    state
        .admin_services()
        .evaluations()
        .retry_judgment(&request.id, &auth.context().mutation_context())
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::ACCEPTED,
        AdminEnvelope::ok(()),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitRequest {
    submission_id: String,
    parameters: EvaluationParameters,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}
fn default_limit() -> u32 {
    20
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryQuery {
    account_ids: String,
    #[serde(default = "history_limit")]
    limit: u32,
}
fn history_limit() -> u32 {
    28
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItemView {
    #[serde(flatten)]
    item: EvaluationHistoryItem,
    tested_at_display: String,
}

async fn account_history<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<HistoryQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let ids = query
        .account_ids
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let items = services
        .evaluations()
        .account_history(&ids, query.limit)
        .await
        .map_err(map_admin_service_error)?;
    let time = TimePresenter::new(services.timezone());
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(
            items
                .into_iter()
                .map(|item| HistoryItemView {
                    tested_at_display: time.datetime(&item.started_at.unwrap_or(item.created_at)),
                    item,
                })
                .collect::<Vec<_>>(),
        ),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanRequest {
    name: String,
    parameters: EvaluationParameters,
    enabled: bool,
    #[serde(default)]
    interval_minutes: Option<u32>,
    #[serde(default)]
    cron_expression: String,
}

impl PlanRequest {
    fn into_settings(
        self,
        timezone: gateway_core::time::DeploymentTimeZone,
    ) -> EvaluationPlanSettings {
        EvaluationPlanSettings {
            name: self.name,
            parameters: self.parameters,
            enabled: self.enabled,
            interval_minutes: self.interval_minutes,
            cron_expression: self.cron_expression,
            timezone,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdatePlanRequest {
    id: String,
    revision: i64,
    settings: PlanRequest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeletePlanRequest {
    id: String,
    revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdRequest {
    id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunView {
    #[serde(flatten)]
    run: EvaluationRun,
    created_at_display: String,
}
impl RunView {
    fn new(run: EvaluationRun, time: TimePresenter) -> Self {
        Self {
            created_at_display: time.datetime(&run.created_at),
            run,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemView {
    #[serde(flatten)]
    item: EvaluationItem,
    started_at_display: Option<String>,
    completed_at_display: Option<String>,
}

#[derive(Serialize)]
struct DetailView {
    run: RunView,
    items: Vec<ItemView>,
}
impl DetailView {
    fn new(detail: EvaluationRunDetail, time: TimePresenter) -> Self {
        Self {
            run: RunView::new(detail.run, time),
            items: detail
                .items
                .into_iter()
                .map(|item| ItemView {
                    started_at_display: item.started_at.as_ref().map(|value| time.datetime(value)),
                    completed_at_display: item
                        .completed_at
                        .as_ref()
                        .map(|value| time.datetime(value)),
                    item,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanView {
    #[serde(flatten)]
    plan: EvaluationPlan,
    next_run_at_display: Option<String>,
}
impl PlanView {
    fn new(plan: EvaluationPlan, time: TimePresenter) -> Self {
        Self {
            next_run_at_display: plan.next_run_at.as_ref().map(|value| time.datetime(value)),
            plan,
        }
    }
}

async fn config<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> impl IntoResponse {
    let tests: Vec<_> = [EvaluationKind::Candy, EvaluationKind::Pelican].into_iter().map(|kind| serde_json::json!({
        "kind": kind, "label": kind.label(), "prompt": kind.prompt(), "promptVersion": kind.prompt_version(), "definition": EvaluationDefinition::preset(kind),
    })).collect();
    AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(serde_json::json!({
            "tests": tests,
            "timezone": state.admin_services().timezone().name(),
            "requestProfileVersion": REQUEST_PROFILE_VERSION,
            "graderVersion": DEFINITION_GRADER_VERSION,
            "limits": { "accounts": MAX_ACCOUNTS_PER_RUN, "repetitions": MAX_REPETITIONS,
                "timeoutSeconds": 600, "queueItems": MAX_QUEUED_ITEMS,
                "parallelRequests": MAX_PARALLEL_REQUESTS, "responseBytes": MAX_RESPONSE_BYTES,
                "artifactBytes": MAX_ARTIFACT_BYTES, "retentionDays": 30,
                "planIntervalMinutes": MAX_PLAN_INTERVAL_MINUTES },
        })),
    )
}

async fn submit<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<SubmitRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let submission_id = Uuid::parse_str(&request.submission_id)
        .map_err(|_| AdminError::bad_request("提交标识必须是 UUID"))?;
    let run = services
        .evaluations()
        .submit(
            submission_id,
            request.parameters,
            &auth.context().mutation_context(),
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::ACCEPTED,
        AdminEnvelope::ok(RunView::new(run, TimePresenter::new(services.timezone()))),
    ))
}

async fn list_runs<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<ListQuery>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let time = TimePresenter::new(services.timezone());
    let runs = services
        .evaluations()
        .list_runs(query.limit, query.offset)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(
            runs.into_iter()
                .map(|run| RunView::new(run, time))
                .collect::<Vec<_>>(),
        ),
    ))
}

async fn detail<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<IdRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let detail = services
        .evaluations()
        .detail(&query.id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(DetailView::new(
            detail,
            TimePresenter::new(services.timezone()),
        )),
    ))
}

async fn stop<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<IdRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let detail = services
        .evaluations()
        .stop(&request.id, &auth.context().mutation_context())
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(DetailView::new(
            detail,
            TimePresenter::new(services.timezone()),
        )),
    ))
}

async fn output<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<IdRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let output = state
        .admin_services()
        .evaluations()
        .output(&query.id)
        .await
        .map_err(map_admin_service_error)?;
    // 永远作为 JSON 返回原文；HTML 的安全处理和无权限 iframe 在受控 UI 内完成
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(output),
    ))
}

async fn list_plans<S: SessionState + Send + Sync>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let time = TimePresenter::new(services.timezone());
    let plans = services
        .evaluations()
        .list_plans()
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(
            plans
                .into_iter()
                .map(|plan| PlanView::new(plan, time))
                .collect::<Vec<_>>(),
        ),
    ))
}

async fn create_plan<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<PlanRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let settings = request.into_settings(services.timezone());
    let plan = services
        .evaluations()
        .save_plan(None, None, settings, &auth.context().mutation_context())
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(PlanView::new(plan, TimePresenter::new(services.timezone()))),
    ))
}

async fn update_plan<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdatePlanRequest>,
) -> Result<impl IntoResponse, AdminError> {
    let services = state.admin_services();
    let settings = request.settings.into_settings(services.timezone());
    let plan = services
        .evaluations()
        .save_plan(
            Some(request.id),
            Some(request.revision),
            settings,
            &auth.context().mutation_context(),
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(PlanView::new(plan, TimePresenter::new(services.timezone()))),
    ))
}

async fn delete_plan<S: SessionState + Send + Sync>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<DeletePlanRequest>,
) -> Result<impl IntoResponse, AdminError> {
    state
        .admin_services()
        .evaluations()
        .delete_plan(
            &request.id,
            request.revision,
            &auth.context().mutation_context(),
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}
