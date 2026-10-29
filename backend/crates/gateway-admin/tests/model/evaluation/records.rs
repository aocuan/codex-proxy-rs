//! 测评参数和计划的边界约束

use gateway_admin::model::evaluation::EvaluationKind;

fn parameters() -> gateway_admin::model::evaluation::EvaluationParameters {
    gateway_admin::model::evaluation::EvaluationParameters {
        definition: None,
        kind: EvaluationKind::Candy,
        client_key_id: "key_test".to_owned(),
        account_ids: vec!["acct_test".to_owned()],
        model: gateway_core::routing::PublicModelId::new("test-model").unwrap(),
        reasoning_effort: None,
        repetitions: 5,
        timeout_seconds: 600,
    }
}

#[test]
fn evaluation_parameters_enforce_experiment_limits_before_enqueuing() {
    let value = parameters();
    assert!(value.validate().is_ok());
    let mut duplicate = value.clone();
    duplicate.account_ids.push("acct_test".to_owned());
    assert!(duplicate.validate().is_err());
    for repetitions in [0, 6] {
        let mut invalid = value.clone();
        invalid.repetitions = repetitions;
        assert!(invalid.validate().is_err());
    }
    let mut pelican = value.clone();
    pelican.kind = EvaluationKind::Pelican;
    assert!(pelican.validate().is_err());
    pelican.repetitions = 1;
    assert!(pelican.validate().is_ok());
    let mut invalid = value;
    invalid.timeout_seconds = 601;
    assert!(invalid.validate().is_err());
}

#[test]
fn plans_validate_five_fields_and_schedule_only_future_slots_in_the_saved_timezone() {
    use chrono::TimeZone as _;
    use gateway_admin::model::evaluation::EvaluationPlanSettings;
    let now = chrono::Utc.with_ymd_and_hms(2026, 10, 20, 2, 0, 0).unwrap();
    let mut settings = EvaluationPlanSettings {
        account_selection: Default::default(),
        name: "每日检查".to_owned(),
        parameters: parameters(),
        enabled: false,
        interval_minutes: None,
        cron_expression: "0 10 * * *".to_owned(),
        timezone: "Asia/Shanghai".parse().unwrap(),
    };
    assert_eq!(settings.next_run_after(now).unwrap(), None);
    settings.enabled = true;
    assert_eq!(
        settings.next_run_after(now).unwrap(),
        Some(now + chrono::Duration::days(1))
    );
    for invalid in ["0 0 10 * * *", "0 10 * * * 2027", "61 * * * *", "* * *"] {
        settings.cron_expression = invalid.to_owned();
        assert!(settings.next_run_after(now).is_err(), "{invalid}");
    }
}

#[test]
fn minute_intervals_keep_their_phase_across_hours_delays_and_timezone_changes() {
    use chrono::{Duration, TimeZone as _};
    use gateway_admin::model::evaluation::EvaluationPlanSettings;
    let now = chrono::Utc
        .with_ymd_and_hms(2026, 10, 20, 2, 40, 17)
        .unwrap();
    let mut settings = EvaluationPlanSettings {
        account_selection: Default::default(),
        name: "间隔检查".to_owned(),
        parameters: parameters(),
        enabled: true,
        interval_minutes: Some(45),
        cron_expression: String::new(),
        timezone: "Asia/Shanghai".parse().unwrap(),
    };
    let first = settings.next_run_after(now).unwrap().unwrap();
    assert_eq!(first, now + Duration::minutes(45));
    for (delay, next_minutes) in [(0, 90), (29, 90), (45 * 60, 135), (90 * 60 + 29, 180)] {
        assert_eq!(
            settings
                .next_occurrence_after(first, first + Duration::seconds(delay))
                .unwrap(),
            now + Duration::minutes(next_minutes)
        );
    }
    settings.timezone = "America/New_York".parse().unwrap();
    assert_eq!(settings.next_run_after(now).unwrap(), Some(first));
    settings.enabled = false;
    assert_eq!(settings.next_run_after(now).unwrap(), None);
    settings.enabled = true;
    assert_eq!(
        settings.next_run_after(now + Duration::days(1)).unwrap(),
        Some(first + Duration::days(1))
    );
    for invalid in [0, 525_601] {
        settings.interval_minutes = Some(invalid);
        assert!(settings.next_run_after(now).is_err());
    }
    settings.interval_minutes = Some(45);
    settings.cron_expression = "* * * * *".to_owned();
    assert!(settings.next_run_after(now).is_err());
}

#[test]
fn automatic_plans_accept_an_empty_pool_while_manual_runs_still_require_accounts() {
    use gateway_admin::model::evaluation::{EvaluationAccountSelection, EvaluationPlanSettings};
    let mut plan = EvaluationPlanSettings {
        account_selection: EvaluationAccountSelection::All,
        name: "自动跟随".to_owned(),
        parameters: parameters(),
        enabled: true,
        interval_minutes: Some(60),
        cron_expression: String::new(),
        timezone: Default::default(),
    };
    assert!(plan.validate_parameters().is_err());
    plan.parameters.account_ids.clear();
    assert!(plan.next_run_after(chrono::Utc::now()).unwrap().is_some());
    assert!(plan.parameters.validate().is_err());
    plan.account_selection = EvaluationAccountSelection::Groups { group_ids: vec![] };
    assert!(plan.validate_parameters().is_err());
    plan.account_selection = EvaluationAccountSelection::Fixed;
    assert!(plan.validate_parameters().is_err());
    plan.enabled = false;
    assert!(plan.validate_parameters().is_ok());
    let mut old = serde_json::to_value(&plan).unwrap();
    old.as_object_mut().unwrap().remove("accountSelection");
    let decoded: EvaluationPlanSettings = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.account_selection, EvaluationAccountSelection::Fixed);
}

#[test]
fn ticket_probe_freezes_two_short_requests_and_cannot_enable_a_judge() {
    let mut probe = parameters();
    probe.kind = EvaluationKind::Turnstate;
    probe.repetitions = 1;
    probe.reasoning_effort = Some("low".into());
    assert!(probe.validate().is_ok());
    let profile =
        gateway_admin::model::evaluation::EvaluationProfile::for_parameters(&probe, "test");
    assert_eq!(profile.prompt, "hi");
    assert_eq!(
        profile.request_profile_version,
        "turnstate-http-two-shots-v1"
    );
    probe.definition =
        Some(gateway_admin::model::evaluation::EvaluationDefinition::preset(EvaluationKind::Candy));
    assert!(probe.validate().is_err());
    probe.definition = None;
    probe.reasoning_effort = Some("high".into());
    assert!(probe.validate().is_err());
}
