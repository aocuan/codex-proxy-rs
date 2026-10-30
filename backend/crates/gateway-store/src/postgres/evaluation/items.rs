//! 子任务领取、输出保存与条件终结

use chrono::{DateTime, Utc};
use gateway_admin::{model::evaluation::*, ports::store::AdminStoreResult};
use sqlx::{PgPool, types::Json};

use super::{conflict, db_error, invalid, lock_queue, rows};

pub(super) async fn claim(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> AdminStoreResult<Option<ClaimedEvaluation>> {
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let running: i64 =
        sqlx::query_scalar("select (select count(*) from evaluation_items where status = 'running') + (select count(*) from evaluation_judgments where status = 'running')")
            .fetch_one(&mut *transaction)
            .await
            .map_err(db_error)?;
    if running >= MAX_PARALLEL_REQUESTS {
        transaction.commit().await.map_err(db_error)?;
        return Ok(None);
    }
    let id: Option<String> = sqlx::query_scalar(
        "select i.id from evaluation_items i join evaluation_runs r on r.id = i.run_id
         where i.status = 'queued' and not r.stop_requested
           and not exists (select 1 from evaluation_items busy
                           where busy.account_id = i.account_id and busy.status = 'running')
           and not exists (select 1 from evaluation_judgments busy
                           where busy.account_id = i.account_id and busy.status = 'running')
         order by r.created_at, r.id, i.id limit 1 for update of i",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    let Some(id) = id else {
        transaction.commit().await.map_err(db_error)?;
        return Ok(None);
    };
    sqlx::query("update evaluation_items set status = 'running', started_at = $2 where id = $1")
        .bind(&id)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    let mut query = rows::item_query();
    query.push("where i.id = ").push_bind(id);
    let item = query
        .build_query_as::<rows::ItemRow>()
        .fetch_one(&mut *transaction)
        .await
        .map_err(db_error)?
        .decode()?;
    let (parameters, profile): (Json<EvaluationParameters>, Json<EvaluationProfile>) =
        sqlx::query_as("select parameters_json, profile_json from evaluation_runs where id = $1")
            .bind(&item.run_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(db_error)?;
    transaction.commit().await.map_err(db_error)?;
    Ok(Some(ClaimedEvaluation {
        item,
        parameters: parameters.0,
        profile: profile.0,
    }))
}

pub(super) async fn save_output(
    pool: &PgPool,
    item_id: &str,
    output: EvaluationOutput,
) -> AdminStoreResult<bool> {
    if output.response_text.len() > MAX_RESPONSE_BYTES
        || output
            .artifact_html
            .as_ref()
            .is_some_and(|html| html.len() > MAX_ARTIFACT_BYTES)
    {
        return Err(invalid("测评输出超过大小限制"));
    }
    let mut transaction = pool.begin().await.map_err(db_error)?;
    let running: Option<String> = sqlx::query_scalar(
        "select id from evaluation_items where id = $1 and status = 'running' for update",
    )
    .bind(item_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    if running.is_none() {
        transaction.commit().await.map_err(db_error)?;
        return Ok(false);
    }
    // 重放相同写入可确认之前的提交，禁止把已消费的输出替换成另一份
    let existing: Option<(String, Option<String>)> = sqlx::query_as(
        "select response_text, artifact_html from evaluation_artifacts where item_id = $1",
    )
    .bind(item_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    if let Some((response, artifact)) = existing {
        if response != output.response_text || artifact != output.artifact_html {
            return Err(conflict("测评输出已保存，不能覆盖"));
        }
    } else {
        sqlx::query(
            "insert into evaluation_artifacts (item_id, response_text, artifact_html) values ($1, $2, $3)",
        )
        .bind(item_id)
        .bind(output.response_text)
        .bind(output.artifact_html)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
    }
    transaction.commit().await.map_err(db_error)?;
    Ok(true)
}

pub(super) async fn finish(
    pool: &PgPool,
    item_id: &str,
    mut completion: EvaluationCompletion,
) -> AdminStoreResult<EvaluationFinish> {
    if !completion.status.is_terminal() || completion.status == EvaluationItemStatus::Skipped {
        return Err(invalid("已领取任务只能迁移到执行终态"));
    }
    if completion.status != EvaluationItemStatus::Succeeded
        && (completion.verdict != EvaluationVerdict::Ungraded || completion.final_answer.is_some())
    {
        return Err(invalid("未成功完成的请求不能判分"));
    }
    let mut transaction = pool.begin().await.map_err(db_error)?;
    lock_queue(&mut transaction).await?;
    let result = sqlx::query(
        "update evaluation_items set status = $2, verdict = $3, final_answer = $4,
            error_code = $5, message = $6, ttft_ms = $7, duration_ms = $8, completed_at = $9,
            observation_json = $10
         where id = $1 and status = 'running'
           and ($2 <> 'succeeded' or exists (select 1 from evaluation_artifacts where item_id = $1))",
    )
    .bind(item_id)
    .bind(completion.status.as_str())
    .bind(completion.verdict.as_str())
    .bind(completion.final_answer)
    .bind(&completion.error_code)
    .bind(&completion.message)
    .bind(completion.ttft_ms)
    .bind(completion.duration_ms)
    .bind(completion.completed_at)
    .bind(Json(&completion.observation))
    .execute(&mut *transaction)
    .await
    .map_err(db_error)?;
    if result.rows_affected() == 1
        && completion.status == EvaluationItemStatus::Succeeded
        && let Some(target) = completion.judge_target
    {
        let stopped: bool = sqlx::query_scalar("select r.stop_requested or i.stop_requested from evaluation_runs r join evaluation_items i on i.run_id = r.id where i.id = $1")
                .bind(item_id).fetch_one(&mut *transaction).await.map_err(db_error)?;
        if !stopped {
            let account = match &target {
                EvaluationJudgeTarget::Account(account) => Some(account.as_str()),
                EvaluationJudgeTarget::TypeSafe => None,
            };
            super::judgments::enqueue(&mut transaction, item_id, account, 1).await?;
        } else {
            sqlx::query(
                "update evaluation_items set message = '批次已停止，未进行 AI 判题' where id = $1",
            )
            .bind(item_id)
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
        }
    }
    let mut config_revision = None;
    if result.rows_affected() == 1
        && let Some(decision) = completion.account_preference
    {
        let (preference, revision) =
            match super::preference::apply(&mut transaction, item_id, decision).await? {
                super::preference::PreferenceApplication::Applied(preference, revision) => {
                    (preference, revision)
                }
                super::preference::PreferenceApplication::Stale(revision) => {
                    transaction.rollback().await.map_err(db_error)?;
                    return Ok(EvaluationFinish {
                        finished: false,
                        config_revision: Some(revision),
                        retry_preference: true,
                    });
                }
            };
        completion.observation.account_preference = Some(preference);
        sqlx::query("update evaluation_items set observation_json = $2 where id = $1")
            .bind(item_id)
            .bind(Json(&completion.observation))
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
        config_revision = revision;
    }
    transaction.commit().await.map_err(db_error)?;
    Ok(EvaluationFinish {
        finished: result.rows_affected() == 1,
        config_revision,
        retry_preference: false,
    })
}
