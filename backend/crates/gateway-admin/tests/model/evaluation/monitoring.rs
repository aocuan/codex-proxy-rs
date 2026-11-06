//! 监测信号的新鲜度、环境隔离、连续确认及未执行任务的展示边界

use chrono::{Duration, Utc};
use gateway_admin::model::evaluation::*;

fn point(
    status: EvaluationItemStatus,
    outcome: Option<TurnStateOutcome>,
    minutes: i64,
) -> EvaluationMonitoringPoint {
    let at = Utc::now() - Duration::minutes(minutes);
    EvaluationMonitoringPoint {
        plan_version: "v1".into(),
        connection_revision: Some("connection".into()),
        request_profile_revision: Some("profile".into()),
        history: EvaluationHistoryItem {
            id: format!("item_{minutes}"),
            run_id: "run".into(),
            account_id: "acct_test".into(),
            name: "正式测票".into(),
            profile_version: "v1".into(),
            grader_type: "none".into(),
            judgment_status: None,
            kind: EvaluationKind::Turnstate,
            model: gateway_core::routing::PublicModelId::new("test-model").unwrap(),
            reasoning_effort: Some("low".into()),
            status,
            verdict: EvaluationVerdict::Ungraded,
            final_answer: None,
            turn_state: outcome,
            duration_ms: Some(10),
            created_at: at,
            started_at: Some(at),
            completed_at: Some(at),
        },
    }
}

fn state(
    points: &[EvaluationMonitoringPoint],
    connection: &str,
    profile: &str,
) -> EvaluationAccountMonitoring {
    let account = EvaluationAccountState {
        credential_state: gateway_core::account::CredentialState::Ready,
        id: "acct_test".into(),
        provider_kind: "openai".into(),
        authentication_kind: "oauth".into(),
        enabled: true,
        group_ids: vec![],
        connection_revision: connection.into(),
    };
    EvaluationAccountMonitoring::derive(
        &account,
        true,
        true,
        &Default::default(),
        &points.iter().collect::<Vec<_>>(),
        profile,
        Utc::now(),
    )
}

#[test]
fn unexecuted_attempts_preserve_the_last_signal_and_failures_are_separate() {
    use EvaluationItemStatus as Status;
    for status in [
        Status::Queued,
        Status::Running,
        Status::Skipped,
        Status::Cancelled,
        Status::Failed,
    ] {
        let points = vec![
            point(status, None, 0),
            point(Status::Succeeded, Some(TurnStateOutcome::Accepted), 1),
            point(Status::Succeeded, Some(TurnStateOutcome::Accepted), 2),
        ];
        let result = state(&points, "connection", "profile");
        assert_eq!(result.signal, Some(TurnStateOutcome::Accepted));
        assert_eq!(result.signal_at, points[1].history.completed_at);
        assert_eq!(result.latest_attempt.unwrap().status, status);
        assert_eq!(
            result.status,
            if status == Status::Failed {
                EvaluationAccountStatus::Failed
            } else {
                EvaluationAccountStatus::Stable
            }
        );
        assert_eq!(
            result.consecutive,
            if status == Status::Failed { 0 } else { 2 }
        );
    }
}

#[test]
fn expired_or_changed_environments_cannot_be_current_stable_accounts() {
    let old = vec![point(
        EvaluationItemStatus::Succeeded,
        Some(TurnStateOutcome::Accepted),
        31,
    )];
    let expired = state(&old, "connection", "profile");
    assert_eq!(expired.status, EvaluationAccountStatus::Stale);
    assert_eq!(expired.freshness, EvaluationFreshness::Expired);
    let recent = vec![point(
        EvaluationItemStatus::Succeeded,
        Some(TurnStateOutcome::Accepted),
        1,
    )];
    for (connection, profile) in [("new-proxy", "profile"), ("connection", "new-client")] {
        let result = state(&recent, connection, profile);
        assert_eq!(result.status, EvaluationAccountStatus::Stale);
        assert_eq!(result.freshness, EvaluationFreshness::EnvironmentChanged);
        assert_eq!(result.signal, None);
    }
}

#[test]
fn changed_and_unknown_signals_interrupt_confirmation_without_becoming_healthy() {
    let points = vec![
        point(
            EvaluationItemStatus::Succeeded,
            Some(TurnStateOutcome::Refreshed),
            0,
        ),
        point(
            EvaluationItemStatus::Succeeded,
            Some(TurnStateOutcome::Accepted),
            1,
        ),
    ];
    let result = state(&points, "connection", "profile");
    assert_eq!(result.status, EvaluationAccountStatus::Changed);
    assert_eq!((result.consecutive, result.required), (1, 2));
    let points = vec![
        point(
            EvaluationItemStatus::Succeeded,
            Some(TurnStateOutcome::Unknown),
            0,
        ),
        points[1].clone(),
    ];
    let result = state(&points, "connection", "profile");
    assert_eq!(result.status, EvaluationAccountStatus::Unknown);
    assert_eq!(result.signal, Some(TurnStateOutcome::Accepted));
    assert_eq!(result.consecutive, 0);
}
