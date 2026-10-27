//! 自动跟随计划与账号生命周期取消的真实仓储和 Worker 行为

use super::*;

async fn due_plan(
    repository: &PgEvaluationRepository,
    selection: EvaluationAccountSelection,
    accounts: &[&str],
) -> EvaluationPlan {
    repository
        .save_plan(
            SaveEvaluationPlan {
                id: "eval_plan_accounts".to_owned(),
                expected_revision: None,
                settings: EvaluationPlanSettings {
                    account_selection: selection,
                    name: "跟随账号".to_owned(),
                    parameters: parameters(accounts, 1),
                    enabled: true,
                    interval_minutes: Some(1),
                    cron_expression: String::new(),
                    timezone: Default::default(),
                },
                next_run_at: Some(Utc::now()),
            },
            &context(),
        )
        .await
        .unwrap()
}

async fn set_enabled(db: &TestDatabase, id: &str, enabled: bool) {
    sqlx::query("update provider_accounts set enabled=$2 where id=$1")
        .bind(id)
        .bind(enabled)
        .execute(&db.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn automatic_plan_waits_for_accounts_freezes_each_run_and_includes_new_accounts() {
    let Some(db) = TestDatabase::create("eval_auto_accounts").await else {
        return;
    };
    let (repository, _, task, service) = setup(&db, Reply::Answer).await;
    sqlx::query("update provider_accounts set enabled=false")
        .execute(&db.pool)
        .await
        .unwrap();
    let plan = due_plan(&repository, EvaluationAccountSelection::All, &[]).await;
    task.advance_schedules(plan.next_run_at.unwrap())
        .await
        .unwrap();
    assert!(repository.list_runs(100, 0).await.unwrap().is_empty());
    let waiting = service.list_plans().await.unwrap().remove(0);
    assert!(waiting.settings.enabled);
    assert_eq!(waiting.available_accounts, Some(0));
    set_enabled(&db, "acct_right", true).await;
    task.advance_schedules(waiting.next_run_at.unwrap())
        .await
        .unwrap();
    let first = repository.list_runs(100, 0).await.unwrap().remove(0);
    assert_eq!(first.parameters.account_ids, ["acct_right"]);
    repository.stop_run(&first.id, &context()).await.unwrap();
    // 新账号在下一轮参加，不能回写已经冻结的名单
    sqlx::query("delete from provider_accounts where id='acct_target'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("insert into provider_accounts (id,provider_kind,name,authentication_kind,provider_credentials_json,has_refresh_token,credential_observed_at,created_at,updated_at) values ('acct_target','openai','new','api_key','{}',false,now(),now(),now()), ('acct_unauthorized','openai','outside','api_key','{}',false,now(),now(),now())").execute(&db.pool).await.unwrap();
    let next = repository.list_plans().await.unwrap().remove(0);
    task.advance_schedules(next.next_run_at.unwrap())
        .await
        .unwrap();
    let runs = repository.list_runs(100, 0).await.unwrap();
    assert_eq!(
        runs[0].parameters.account_ids,
        ["acct_right", "acct_target"]
    );
    assert_eq!(
        repository
            .load_run(&first.id)
            .await
            .unwrap()
            .unwrap()
            .run
            .parameters
            .account_ids,
        ["acct_right"]
    );
    db.close().await;
}

#[tokio::test]
async fn account_disable_and_delete_stop_only_affected_work_and_never_replay_on_enable() {
    for delete in [false, true] {
        let Some(db) = TestDatabase::create("eval_account_stop").await else {
            return;
        };
        let (repository, execution, task, service) = setup(&db, Reply::Pending).await;
        let run = service
            .submit(
                uuid::Uuid::new_v4(),
                parameters(&["acct_pending"], 3),
                &context(),
            )
            .await
            .unwrap();
        let (cancel, handle) = start(task);
        wait_started(&execution).await;
        if delete {
            sqlx::query("delete from provider_accounts where id='acct_pending'")
                .execute(&db.pool)
                .await
                .unwrap();
        } else {
            set_enabled(&db, "acct_pending", false).await;
        }
        let detail = wait_finished(&repository, &run.id).await;
        assert!(!detail.run.stop_requested);
        assert_eq!(
            detail
                .items
                .iter()
                .filter(|i| i.status == EvaluationItemStatus::Cancelled)
                .count(),
            1
        );
        assert_eq!(
            detail
                .items
                .iter()
                .filter(|i| i.status == EvaluationItemStatus::Skipped)
                .count(),
            2
        );
        assert_eq!(execution.cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(execution.commits.load(Ordering::SeqCst), 0);
        set_enabled(&db, "acct_pending", true).await;
        assert!(repository.claim_next(Utc::now()).await.unwrap().is_none());
        // 其他账号仍可加入队列，整轮停止标记未被借用
        let other = service
            .submit(
                uuid::Uuid::new_v4(),
                parameters(&["acct_right"], 1),
                &context(),
            )
            .await
            .unwrap();
        assert!(
            !repository
                .is_work_stop_requested(
                    &repository.load_run(&other.id).await.unwrap().unwrap().items[0].id,
                    None
                )
                .await
                .unwrap()
        );
        cancel.cancel();
        handle.await.unwrap().unwrap();
        db.close().await;
    }
}

#[tokio::test]
async fn group_removal_cancels_queued_work_and_fixed_plans_drop_deleted_accounts() {
    let Some(db) = TestDatabase::create("eval_group_accounts").await else {
        return;
    };
    let (repository, _, task, _) = setup(&db, Reply::Answer).await;
    let group = "grp_11111111111111111111111111111111";
    sqlx::query("insert into account_groups (id,name,color,created_at,updated_at) values ($1,'test','#FFFFFFFF',now(),now())").bind(group).execute(&db.pool).await.unwrap();
    sqlx::query("insert into account_group_accounts (account_group_id,provider_account_id,created_at) values ($1,'acct_right',now())").bind(group).execute(&db.pool).await.unwrap();
    let plan = due_plan(
        &repository,
        EvaluationAccountSelection::Groups {
            group_ids: vec![group.to_owned()],
        },
        &[],
    )
    .await;
    task.advance_schedules(plan.next_run_at.unwrap())
        .await
        .unwrap();
    let run = repository.list_runs(10, 0).await.unwrap().remove(0);
    assert_eq!(run.parameters.account_ids, ["acct_right"]);
    sqlx::query("delete from account_group_accounts where account_group_id=$1")
        .bind(group)
        .execute(&db.pool)
        .await
        .unwrap();
    repository.reconcile_accounts().await.unwrap();
    assert_eq!(
        repository.load_run(&run.id).await.unwrap().unwrap().items[0].status,
        EvaluationItemStatus::Skipped
    );
    repository
        .delete_plan(&plan.id, plan.revision, &context())
        .await
        .unwrap();
    let fixed = due_plan(
        &repository,
        EvaluationAccountSelection::Fixed,
        &["acct_right"],
    )
    .await;
    sqlx::query("delete from provider_accounts where id='acct_right'")
        .execute(&db.pool)
        .await
        .unwrap();
    task.advance_schedules(fixed.next_run_at.unwrap())
        .await
        .unwrap();
    let fixed = repository.list_plans().await.unwrap().remove(0);
    assert!(!fixed.settings.enabled);
    assert!(fixed.settings.parameters.account_ids.is_empty());
    assert!(fixed.next_run_at.is_none());
    db.close().await;
}

#[tokio::test]
async fn disabling_judge_cancels_grading_without_erasing_successful_answer() {
    let Some(db) = TestDatabase::create("eval_judge_disabled").await else {
        return;
    };
    let (repository, execution, task, service) = setup(&db, Reply::JudgePending).await;
    let run = service
        .submit(uuid::Uuid::new_v4(), model_parameters(), &context())
        .await
        .unwrap();
    let (cancel, handle) = start(task);
    tokio::time::timeout(Duration::from_secs(15), async {
        while execution.accounts.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    set_enabled(&db, "acct_judge", false).await;
    let detail = wait_finished(&repository, &run.id).await;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    assert_eq!(detail.items[0].status, EvaluationItemStatus::Succeeded);
    assert_eq!(
        detail.items[0].judgment.as_ref().unwrap().status,
        EvaluationItemStatus::Cancelled
    );
    assert!(
        repository
            .load_output(&detail.items[0].id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(execution.commits.load(Ordering::SeqCst), 1);
    assert_eq!(execution.cleanups.load(Ordering::SeqCst), 2);
    db.close().await;
}
