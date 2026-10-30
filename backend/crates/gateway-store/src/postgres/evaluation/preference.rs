//! 测评终态、目标分组成员关系与配置审计在同一事务中提交

use gateway_admin::{
    model::{
        MutationActor, MutationContext, Revision, audit::MutationAuditOperation, evaluation::*,
    },
    ports::store::AdminStoreResult,
};
use sqlx::{Postgres, Transaction, types::Json};

use super::{db_error, invalid};

pub(super) enum PreferenceApplication {
    Applied(EvaluationPreferenceResult, Option<Revision>),
    Stale(Revision),
}

pub(super) async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    item_id: &str,
    decision: EvaluationPreferenceDecision,
) -> AdminStoreResult<PreferenceApplication> {
    let mut result = EvaluationPreferenceResult {
        group_id: decision.rule.group_id.clone(),
        status: EvaluationPreferenceStatus::Skipped,
        reason: None,
    };
    let Some(desired) = decision.desired_member else {
        result.reason = decision
            .skip_reason
            .or_else(|| Some("测票未得出明确结果".into()));
        return Ok(PreferenceApplication::Applied(result, None));
    };
    let row: (String, Option<String>, bool) = sqlx::query_as(
        "select i.account_id, r.plan_id, i.stop_requested or r.stop_requested
         from evaluation_items i join evaluation_runs r on r.id = i.run_id where i.id = $1",
    )
    .bind(item_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)?;
    let (account_id, plan_id, stopped) = row;
    if stopped || plan_id.is_none() {
        result.reason = Some("测评已停止或计划已删除".into());
        return Ok(PreferenceApplication::Applied(result, None));
    }
    let plan: Option<(i64, Json<EvaluationPlanSettings>)> = sqlx::query_as(
        "select revision, settings_json from evaluation_plans where id = $1 for update",
    )
    .bind(plan_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?;
    let Some((revision, Json(settings))) = plan else {
        result.reason = Some("计划已删除".into());
        return Ok(PreferenceApplication::Applied(result, None));
    };
    if revision != decision.rule.plan_revision
        || !settings.enabled
        || !settings
            .account_preference
            .as_ref()
            .is_some_and(|rule| rule.group_id == decision.rule.group_id)
    {
        result.reason = Some("计划已暂停或配置已变化".into());
        return Ok(PreferenceApplication::Applied(result, None));
    }
    // 与人工账号修改共享配置版本锁，避免覆盖并发分组修改
    let current: i64 =
        sqlx::query_scalar("select config_revision from runtime_settings where id = 1 for update")
            .fetch_one(&mut **tx)
            .await
            .map_err(db_error)?;
    let current = Revision::new(u64::try_from(current).map_err(|_| invalid("优选配置版本无效"))?)
        .map_err(|_| invalid("优选配置版本无效"))?;
    let Some(expected) = decision.expected_revision else {
        result.reason = Some("当前账号授权配置不可用".into());
        return Ok(PreferenceApplication::Applied(result, None));
    };
    if current != expected {
        return Ok(PreferenceApplication::Stale(current));
    }
    let account: Option<bool> =
        sqlx::query_scalar("select enabled from provider_accounts where id = $1 for update")
            .bind(&account_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
    let group: Option<bool> =
        sqlx::query_scalar("select enabled from account_groups where id = $1 for update")
            .bind(&decision.rule.group_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_error)?;
    if account != Some(true) || group != Some(true) {
        result.reason = Some("账号或优选分组已删除或停用".into());
        return Ok(PreferenceApplication::Applied(result, None));
    }
    if let EvaluationAccountSelection::Groups { group_ids } = &settings.account_selection {
        let in_source: bool = sqlx::query_scalar(
            "select exists(select 1 from account_group_accounts where provider_account_id = $1 and account_group_id = any($2::text[]))",
        ).bind(&account_id).bind(group_ids).fetch_one(&mut **tx).await.map_err(db_error)?;
        if !in_source {
            result.reason = Some("账号已移出检测范围".into());
            return Ok(PreferenceApplication::Applied(result, None));
        }
    }
    let changed = if desired {
        sqlx::query("insert into account_group_accounts (account_group_id, provider_account_id, created_at) values ($1, $2, now()) on conflict do nothing")
            .bind(&decision.rule.group_id).bind(&account_id)
            .execute(&mut **tx).await.map_err(db_error)?.rows_affected() > 0
    } else {
        sqlx::query("delete from account_group_accounts where account_group_id = $1 and provider_account_id = $2")
            .bind(&decision.rule.group_id).bind(&account_id)
            .execute(&mut **tx).await.map_err(db_error)?.rows_affected() > 0
    };
    if !changed {
        result.status = EvaluationPreferenceStatus::Unchanged;
        return Ok(PreferenceApplication::Applied(result, None));
    }
    result.status = if desired {
        EvaluationPreferenceStatus::Joined
    } else {
        EvaluationPreferenceStatus::Removed
    };
    let revision = super::super::bump_config_revision_in_transaction(tx)
        .await
        .map_err(|error| crate::admin_store_error("evaluation preference", error))?;
    let context = MutationContext {
        actor: MutationActor::System,
        request_id: item_id.to_owned(),
    };
    let event = crate::mutation_audit(
        &context,
        MutationAuditOperation::EvaluationAccountPreference { joined: desired },
        &account_id,
        vec!["group_ids".into()],
    );
    super::super::append_admin_audit_event_in_transaction(tx, event, revision)
        .await
        .map_err(|error| crate::admin_store_error("evaluation preference", error))?;
    let event = crate::mutation_audit(
        &context,
        MutationAuditOperation::EvaluationGroupPreference { joined: desired },
        &decision.rule.group_id,
        vec!["member_ids".into()],
    );
    super::super::append_admin_audit_event_in_transaction(tx, event, revision)
        .await
        .map_err(|error| crate::admin_store_error("evaluation preference", error))?;
    Ok(PreferenceApplication::Applied(
        result,
        Some(crate::admin_revision(revision)?),
    ))
}
