//! 两请求测票的有效性、路由变化与判题隔离

use gateway_admin::model::evaluation::{TurnStateOutcome, TurnStateProbeResult, TurnStateStep};

fn step(ticket: &str, completed: bool, changed: bool) -> TurnStateStep {
    TurnStateStep {
        status: 200,
        duration_ms: 100,
        ticket_length: ticket.len(),
        ticket_fingerprint: ticket.into(),
        route_changed: changed,
        completed,
    }
}

#[test]
fn ticket_echo_reports_only_stability_and_never_confuses_errors_with_acceptance() {
    let cases = [
        (
            vec![step("A", true, false), step("A", true, false)],
            None,
            TurnStateOutcome::Accepted,
        ),
        (
            vec![step("A", true, false), step("", true, false)],
            None,
            TurnStateOutcome::Accepted,
        ),
        (
            vec![step("A", true, false), step("B", true, false)],
            None,
            TurnStateOutcome::Refreshed,
        ),
        (
            vec![step("A", true, false), step("A", true, true)],
            None,
            TurnStateOutcome::Unknown,
        ),
        (
            vec![step("A", true, false), step("B", false, false)],
            None,
            TurnStateOutcome::Unknown,
        ),
        (vec![step("", true, false)], None, TurnStateOutcome::Unknown),
        (
            vec![step("A", true, false)],
            Some("network".into()),
            TurnStateOutcome::Unknown,
        ),
    ];
    for (steps, error, expected) in cases {
        assert_eq!(TurnStateProbeResult::new(steps, error).outcome, expected);
    }
}
