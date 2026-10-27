//! 定时测评配置的版本条件写入与审计

use gateway_admin::{
    model::{MutationContext, audit::MutationAuditOperation, evaluation::*},
    ports::store::{AdminStoreError, AdminStoreErrorKind, AdminStoreResult},
};
use sqlx::{PgPool, types::Json};

use super::{append_audit, db_error, invalid, rows::PlanRow};

pub(super) async fn list(pool: &PgPool) -> AdminStoreResult<Vec<EvaluationPlan>> {
    Ok(sqlx::query_as::<_, PlanRow>(
        "select id, revision, settings_json, next_run_at, created_at, updated_at
         from evaluation_plans order by created_at desc, id desc",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?
    .into_iter()
    .map(Into::into)
    .collect())
}

pub(super) async fn save(
    pool: &PgPool,
    command: SaveEvaluationPlan,
    context: &MutationContext,
) -> AdminStoreResult<EvaluationPlan> {
    command
        .settings
        .validate_parameters()
        .map_err(|source| invalid("测评计划参数无效").with_source(source))?;
    if command.settings.enabled != command.next_run_at.is_some() {
        return Err(invalid(
            "启用计划必须具有下次执行时间，停用计划不能保留游标",
        ));
    }
    let mut transaction = pool.begin().await.map_err(db_error)?;
    let operation;
    let plan: Option<PlanRow> = if let Some(revision) = command.expected_revision {
        operation = MutationAuditOperation::EvaluationPlanUpdate;
        sqlx::query_as(
            "update evaluation_plans
             set revision = revision + 1, settings_json = $3, next_run_at = $4, updated_at = now()
             where id = $1 and revision = $2
             returning id, revision, settings_json, next_run_at, created_at, updated_at",
        )
        .bind(&command.id)
        .bind(revision)
        .bind(Json(command.settings))
        .bind(command.next_run_at)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db_error)?
    } else {
        operation = MutationAuditOperation::EvaluationPlanCreate;
        sqlx::query_as(
            "insert into evaluation_plans (id, settings_json, next_run_at, created_at, updated_at)
             values ($1, $2, $3, now(), now()) on conflict (id) do nothing
             returning id, revision, settings_json, next_run_at, created_at, updated_at",
        )
        .bind(&command.id)
        .bind(Json(command.settings))
        .bind(command.next_run_at)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db_error)?
    };
    let plan = plan.ok_or_else(stale_revision)?;
    append_audit(
        &mut transaction,
        context,
        operation,
        &command.id,
        &["settings", "next_run_at"],
    )
    .await?;
    transaction.commit().await.map_err(db_error)?;
    Ok(plan.into())
}

pub(super) async fn delete(
    pool: &PgPool,
    id: &str,
    expected_revision: i64,
    context: &MutationContext,
) -> AdminStoreResult<bool> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    let result = sqlx::query("delete from evaluation_plans where id = $1 and revision = $2")
        .bind(id)
        .bind(expected_revision)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        let exists: bool =
            sqlx::query_scalar("select exists(select 1 from evaluation_plans where id = $1)")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(db_error)?;
        if exists {
            return Err(stale_revision());
        }
        transaction.commit().await.map_err(db_error)?;
        return Ok(false);
    }
    append_audit(
        &mut transaction,
        context,
        MutationAuditOperation::EvaluationPlanDelete,
        id,
        &["deleted"],
    )
    .await?;
    transaction.commit().await.map_err(db_error)?;
    Ok(true)
}

fn stale_revision() -> AdminStoreError {
    AdminStoreError::new(
        AdminStoreErrorKind::StaleRevision,
        "evaluation",
        "测评计划已变化，请刷新后重试",
    )
}
