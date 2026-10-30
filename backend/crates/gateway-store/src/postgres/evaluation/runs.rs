//! 测评提交、停止与计划触发的原子事务

use chrono::{DateTime, Utc};
use gateway_admin::{
    model::{MutationActor, MutationContext, audit::MutationAuditOperation, evaluation::*},
    ports::store::AdminStoreResult,
};
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction, types::Json};

use super::{append_audit, conflict, db_error, invalid, lock_queue, rows};

pub(super) async fn enqueue_manual(
    pool: &PgPool,
    submission_id: uuid::Uuid,
    seed: EvaluationRunSeed,
    context: &MutationContext,
) -> AdminStoreResult<EvaluationRun> {
    validate_seed(&seed)?;
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let mut query = rows::run_query();
    query
        .push("where r.actor_json = ")
        .push_bind(Json(context.actor.clone()))
        .push(" and r.submission_id = ")
        .push_bind(submission_id);
    if let Some(existing) = query
        .build_query_as::<rows::RunRow>()
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db_error)?
    {
        let existing: EvaluationRun = existing.into();
        if existing.parameters != seed.parameters {
            return Err(conflict("同一提交标识不能用于不同测评参数"));
        }
        transaction.commit().await.map_err(db_error)?;
        return Ok(existing);
    }
    if !has_capacity(&mut transaction, seed.parameters.item_count()).await? {
        return Err(conflict("测评等待队列已满，请稍后再试"));
    }
    insert_run(
        &mut transaction,
        &seed,
        &context.actor,
        Some(submission_id),
        None,
        None,
    )
    .await?;
    append_audit(
        &mut transaction,
        context,
        MutationAuditOperation::EvaluationRunCreate,
        &seed.id,
        &["parameters"],
    )
    .await?;
    let run = read_run(&mut transaction, &seed.id)
        .await?
        .ok_or_else(|| invalid("测评记录不存在"))?;
    transaction.commit().await.map_err(db_error)?;
    Ok(run)
}

pub(super) async fn enqueue_scheduled(
    pool: &PgPool,
    command: EnqueueScheduledEvaluation,
) -> AdminStoreResult<bool> {
    command
        .seed
        .parameters
        .validate_template()
        .map_err(|e| invalid("测评参数无效").with_source(e))?;
    if !command.seed.parameters.account_ids.is_empty() {
        validate_seed(&command.seed)?;
    }
    if command.next_run_at <= command.scheduled_at {
        return Err(invalid("计划游标必须向前推进"));
    }
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let current: Option<(Json<EvaluationPlanSettings>,)> = sqlx::query_as(
        "select settings_json from evaluation_plans
         where id = $1 and revision = $2 and next_run_at = $3
           and settings_json @> '{\"enabled\": true}'::jsonb for update",
    )
    .bind(&command.plan_id)
    .bind(command.expected_revision)
    .bind(command.scheduled_at)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    let Some((settings,)) = current else {
        transaction.commit().await.map_err(db_error)?;
        return Ok(false);
    };
    let mut template = command.seed.parameters.clone();
    template
        .account_ids
        .clone_from(&settings.parameters.account_ids);
    if settings.parameters != template
        || settings.account_selection != command.seed.profile.account_selection
        || settings
            .account_preference
            .as_ref()
            .map(|rule| (&rule.group_id, command.expected_revision))
            != command
                .seed
                .profile
                .account_preference
                .as_ref()
                .map(|rule| (&rule.group_id, rule.plan_revision))
    {
        return Err(conflict("测评计划参数已变化"));
    }
    if command.seed.parameters.account_ids.is_empty() {
        sqlx::query("update evaluation_plans set next_run_at=$2 where id=$1")
            .bind(&command.plan_id)
            .bind(command.next_run_at)
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        return Ok(true);
    }
    let active: bool = sqlx::query_scalar(
        "select exists (
            select 1 from evaluation_runs r join evaluation_items i on i.run_id = r.id
            where r.plan_id = $1 and (i.status in ('queued', 'running') or exists (select 1 from evaluation_judgments j where j.item_id = i.id and j.status in ('queued', 'running')))
         )",
    )
    .bind(&command.plan_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(db_error)?;
    let skip_reason = if command.skip_reason.is_some() {
        command.skip_reason
    } else if active {
        Some("上一轮测评尚未结束".to_owned())
    } else if !has_capacity(&mut transaction, command.seed.parameters.item_count()).await? {
        Some("测评等待队列已满".to_owned())
    } else {
        None
    };
    insert_run(
        &mut transaction,
        &command.seed,
        &MutationActor::System,
        None,
        Some((&command.plan_id, command.scheduled_at)),
        skip_reason.as_deref(),
    )
    .await?;
    sqlx::query("update evaluation_plans set next_run_at = $2 where id = $1")
        .bind(&command.plan_id)
        .bind(command.next_run_at)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    transaction.commit().await.map_err(db_error)?;
    Ok(true)
}

pub(super) async fn list(
    pool: &PgPool,
    limit: u32,
    offset: u32,
) -> AdminStoreResult<Vec<EvaluationRun>> {
    let mut query = rows::run_query();
    query
        .push("order by r.created_at desc, r.id desc limit ")
        .push_bind(i64::from(limit.clamp(1, 100)))
        .push(" offset ")
        .push_bind(i64::from(offset));
    Ok(query
        .build_query_as::<rows::RunRow>()
        .fetch_all(pool)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(Into::into)
        .collect())
}

pub(super) async fn load(pool: &PgPool, id: &str) -> AdminStoreResult<Option<EvaluationRunDetail>> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    // 同一次详情读取不能混用领取前的计数与领取后的子任务
    sqlx::query("set transaction isolation level repeatable read read only")
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    let Some(run) = read_run(&mut transaction, id).await? else {
        transaction.commit().await.map_err(db_error)?;
        return Ok(None);
    };
    let mut query = rows::item_query();
    query
        .push("where i.run_id = ")
        .push_bind(id.to_owned())
        .push(" order by i.id");
    let items = query
        .build_query_as::<rows::ItemRow>()
        .fetch_all(&mut *transaction)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(rows::ItemRow::decode)
        .collect::<AdminStoreResult<Vec<_>>>()?;
    transaction.commit().await.map_err(db_error)?;
    Ok(Some(EvaluationRunDetail { run, items }))
}

pub(super) async fn stop(
    pool: &PgPool,
    id: &str,
    context: &MutationContext,
) -> AdminStoreResult<bool> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let changed = sqlx::query(
        "update evaluation_runs set stop_requested = true where id = $1 and not stop_requested
         and exists (select 1 from evaluation_items i where run_id = $1 and (status in ('queued', 'running') or exists (select 1 from evaluation_judgments j where j.item_id = i.id and j.status in ('queued', 'running'))))",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await
    .map_err(db_error)?
    .rows_affected()
        == 1;
    if changed {
        sqlx::query("update evaluation_judgments j set status = 'cancelled', reason = '管理员已停止本轮评审', completed_at = now()
            from evaluation_items i where i.id = j.item_id and i.run_id = $1 and j.status = 'queued'")
            .bind(id).execute(&mut *transaction).await.map_err(db_error)?;
        sqlx::query(
            "update evaluation_items set status = 'skipped', completed_at = now(),
                error_code = 'evaluation.stopped', message = '管理员已停止本轮测评'
             where run_id = $1 and status = 'queued'",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
        append_audit(
            &mut transaction,
            context,
            MutationAuditOperation::EvaluationRunStop,
            id,
            &["stop_requested"],
        )
        .await?;
    }
    transaction.commit().await.map_err(db_error)?;
    Ok(changed)
}

async fn read_run(
    transaction: &mut Transaction<'_, Postgres>,
    id: &str,
) -> AdminStoreResult<Option<EvaluationRun>> {
    let mut query = rows::run_query();
    query.push("where r.id = ").push_bind(id.to_owned());
    Ok(query
        .build_query_as::<rows::RunRow>()
        .fetch_optional(&mut **transaction)
        .await
        .map_err(db_error)?
        .map(Into::into))
}

pub(super) async fn has_capacity(
    transaction: &mut Transaction<'_, Postgres>,
    item_count: usize,
) -> AdminStoreResult<bool> {
    // 将生成和判题视作同一任务链，生成结束转入判题时不额外占用名额
    let pending: i64 = sqlx::query_scalar(
        "select
            (select count(*) from evaluation_items where status in ('queued', 'running')) +
            (select count(*) from evaluation_judgments where status in ('queued', 'running'))",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(db_error)?;
    let count = i64::try_from(item_count).map_err(|_| invalid("测评数量过大"))?;
    Ok(pending + count <= MAX_QUEUED_ITEMS)
}

async fn insert_run(
    transaction: &mut Transaction<'_, Postgres>,
    seed: &EvaluationRunSeed,
    actor: &MutationActor,
    submission_id: Option<uuid::Uuid>,
    schedule: Option<(&str, DateTime<Utc>)>,
    skip_reason: Option<&str>,
) -> AdminStoreResult<()> {
    sqlx::query(
        "insert into evaluation_runs
         (id, parameters_json, profile_json, actor_json, submission_id, plan_id, scheduled_at, created_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&seed.id)
    .bind(Json(&seed.parameters))
    .bind(Json(&seed.profile))
    .bind(Json(actor))
    .bind(submission_id)
    .bind(schedule.map(|value| value.0))
    .bind(schedule.map(|value| value.1))
    .bind(seed.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(db_error)?;
    let mut query = QueryBuilder::<Postgres>::new(
        "insert into evaluation_items (id, run_id, account_id, repetition, status,
            completed_at, error_code, message) ",
    );
    let items = seed.parameters.account_ids.iter().flat_map(|account_id| {
        (1..=seed.parameters.repetitions).map(move |repetition| (account_id, repetition))
    });
    query.push_values(items, |mut row, (account_id, repetition)| {
        row.push_bind(format!("eval_item_{}", uuid::Uuid::now_v7().simple()))
            .push_bind(&seed.id)
            .push_bind(account_id)
            .push_bind(i32::from(repetition))
            .push_bind(if skip_reason.is_some() {
                "skipped"
            } else {
                "queued"
            })
            .push_bind(skip_reason.map(|_| seed.created_at))
            .push_bind(skip_reason.map(|_| "evaluation.schedule_skipped"))
            .push_bind(skip_reason);
    });
    query
        .build()
        .execute(&mut **transaction)
        .await
        .map_err(db_error)?;
    Ok(())
}

fn validate_seed(seed: &EvaluationRunSeed) -> AdminStoreResult<()> {
    seed.parameters
        .validate()
        .map_err(|source| invalid("测评参数无效").with_source(source))
}
