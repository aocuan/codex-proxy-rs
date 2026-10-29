//! 两请求测票的诊断事实与判据，不把路由信号作为能力题评分

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStateOutcome {
    Accepted,
    Refreshed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStateStep {
    pub status: u16,
    pub duration_ms: u64,
    pub ticket_length: usize,
    pub ticket_fingerprint: String,
    pub route_changed: bool,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStateProbeResult {
    pub outcome: TurnStateOutcome,
    pub steps: Vec<TurnStateStep>,
    pub error: Option<String>,
}

impl TurnStateProbeResult {
    #[must_use]
    pub fn new(steps: Vec<TurnStateStep>, error: Option<String>) -> Self {
        let outcome = match steps.as_slice() {
            [first, second]
                if error.is_none()
                    && first.completed
                    && second.completed
                    && first.ticket_length > 0
                    && !second.route_changed =>
            {
                if second.ticket_length == 0
                    || first.ticket_fingerprint == second.ticket_fingerprint
                {
                    TurnStateOutcome::Accepted
                } else {
                    TurnStateOutcome::Refreshed
                }
            }
            _ => TurnStateOutcome::Unknown,
        };
        Self {
            outcome,
            steps,
            error,
        }
    }

    #[must_use]
    pub fn summary(&self) -> &str {
        if let Some(error) = &self.error {
            return error;
        }
        match self.outcome {
            TurnStateOutcome::Accepted => "票据回带后保持稳定，仅代表本次路由信号正常",
            TurnStateOutcome::Refreshed => "回带后上游换发新票，仅为路由变化线索，需能力评测复核",
            TurnStateOutcome::Unknown if self.steps.iter().any(|step| step.route_changed) => {
                "负载均衡路由发生变化，测票未判定"
            }
            TurnStateOutcome::Unknown => "上游未签发票据，测票未判定",
        }
    }
}
