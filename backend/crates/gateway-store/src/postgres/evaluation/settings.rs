//! 测评默认设置的乐观锁与敏感凭据写入，响应仅返回配置状态

use gateway_admin::{
    model::{MutationContext, audit::MutationAuditOperation, evaluation::*},
    ports::store::{AdminStoreError, AdminStoreErrorKind, AdminStoreResult},
};
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::{PgPool, types::Json};

use super::{append_audit, db_error, invalid};

pub(super) async fn load(pool: &PgPool) -> AdminStoreResult<EvaluationSettings> {
    let row: Option<(i64, Json<EvaluationDefaults>, bool)> = sqlx::query_as(
        "select revision, defaults_json, typesafe_api_key is not null from evaluation_settings where id = 1",
    ).fetch_optional(pool).await.map_err(db_error)?;
    Ok(row.map_or_else(
        EvaluationSettings::default,
        |(revision, defaults, typesafe_configured)| EvaluationSettings {
            revision,
            defaults: defaults.0,
            typesafe_configured,
        },
    ))
}

pub(super) async fn key(pool: &PgPool) -> AdminStoreResult<Option<SecretString>> {
    let value: Option<Option<String>> =
        sqlx::query_scalar("select typesafe_api_key from evaluation_settings where id = 1")
            .fetch_optional(pool)
            .await
            .map_err(db_error)?;
    Ok(value.flatten().map(SecretString::from))
}

pub(super) async fn save(
    pool: &PgPool,
    command: UpdateEvaluationSettings,
    context: &MutationContext,
) -> AdminStoreResult<EvaluationSettings> {
    command
        .validate()
        .map_err(|error| invalid("测评设置无效").with_source(error))?;
    let mut transaction = pool.begin().await.map_err(db_error)?;
    sqlx::query("select pg_advisory_xact_lock(hashtextextended(current_schema() || ':evaluation_settings', 0))")
        .execute(&mut *transaction).await.map_err(db_error)?;
    let current: Option<(i64, Option<String>)> = sqlx::query_as(
        "select revision, typesafe_api_key from evaluation_settings where id = 1 for update",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(db_error)?;
    let (revision, existing_key) = current.unwrap_or((0, None));
    if revision != command.revision {
        return Err(AdminStoreError::new(
            AdminStoreErrorKind::StaleRevision,
            "evaluation",
            "测评设置已变化，请重新打开后修改",
        ));
    }
    let key = command
        .typesafe_api_key
        .as_ref()
        .map_or(existing_key, |value| {
            let key = value.expose_secret();
            (!key.is_empty()).then(|| key.to_owned())
        });
    if matches!(
        command.defaults.judge,
        Some(EvaluationJudgeDefaults::Typesafe { .. })
    ) && key.is_none()
    {
        return Err(invalid("请先填写 TypeSafe API Key"));
    }
    let revision = revision
        .checked_add(1)
        .ok_or_else(|| invalid("测评设置版本超出范围"))?;
    sqlx::query(
        "insert into evaluation_settings (id, revision, defaults_json, typesafe_api_key)
         values (1, $1, $2, $3) on conflict (id) do update
         set revision = excluded.revision, defaults_json = excluded.defaults_json, typesafe_api_key = excluded.typesafe_api_key",
    ).bind(revision).bind(Json(&command.defaults)).bind(&key)
        .execute(&mut *transaction).await.map_err(db_error)?;
    let mut fields = vec!["defaults"];
    if command.typesafe_api_key.is_some() {
        fields.push("typesafe_credential");
    }
    append_audit(
        &mut transaction,
        context,
        MutationAuditOperation::EvaluationSettingsUpdate,
        "evaluation_settings",
        &fields,
    )
    .await?;
    transaction.commit().await.map_err(db_error)?;
    Ok(EvaluationSettings {
        revision,
        defaults: command.defaults,
        typesafe_configured: key.is_some(),
    })
}
