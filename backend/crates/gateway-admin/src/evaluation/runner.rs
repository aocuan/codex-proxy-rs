//! 单个测评的保存、消费确认与取消收尾，任何错误路径都保留会话清理责任

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use chrono::Utc;
use gateway_core::{
    account::ProviderAccountId,
    engine::{
        EngineError,
        execution::{
            ClientTransport, ExecutionRequestMetadata, ExecutionService, StartAccountExecution,
            gateway_error_from_engine,
        },
    },
    error::{GatewayError, GatewayErrorKind},
    lifecycle::CancellationToken,
    operation::{GenerateRequest, Operation, ProtocolPayload},
    policy::ClientApiKeyId,
    task::WorkerTaskError,
};
use serde_json::json;

use super::collector::EvaluationCollector;
use crate::{
    model::{evaluation::*, observability::UsageRecord},
    ports::{
        evaluation::EvaluationRepository,
        store::{AdminStoreError, AdminStoreErrorKind, ObservabilityStore},
    },
};

pub(super) struct EvaluationRunner {
    pub providers: Option<crate::ports::provider::ProviderAdminRegistry>,
    pub publisher: Option<Arc<dyn gateway_core::runtime::SnapshotControl>>,
    pub snapshot: gateway_core::runtime::RuntimeSnapshotHandle,
    pub typesafe: Option<std::sync::Arc<dyn crate::ports::evaluation::TypeSafeJudgeClient>>,
    pub repository: Arc<dyn EvaluationRepository>,
    pub execution: Arc<dyn ExecutionService>,
    pub observability: Arc<dyn ObservabilityStore>,
}

impl EvaluationRunner {
    pub async fn run(
        &self,
        claim: ClaimedEvaluation,
        parent: CancellationToken,
    ) -> Result<(), WorkerTaskError> {
        if claim.profile.account_preference.is_some() && self.publisher.is_none() {
            return Err(WorkerTaskError::safe("账号优选的配置发布服务不可用"));
        }
        let cancellation = parent.child_token();
        let execution = self.execute(&claim, cancellation.clone());
        tokio::pin!(execution);
        let stop = self.watch_stop(&claim.item.id, None, &parent);
        tokio::pin!(stop);
        let mut result = tokio::select! {
            result = &mut execution => result?,
            stopped = &mut stop => {
                cancellation.cancel();
                let result = execution.await;
                // 已开始的会话必须完成收尾，之后才向 Host 报告存储故障
                stopped.map_err(store_error)?;
                result?
            }
        };
        result.completed_at = Utc::now();
        let mut attempts = 0;
        loop {
            result.account_preference = claim
                .profile
                .account_preference
                .as_ref()
                .map(|rule| self.preference_decision(rule, &claim, &result, attempts >= 3));
            let finished = self
                .repository
                .finish_item(&claim.item.id, result.clone())
                .await
                .map_err(store_error)?;
            if let (Some(revision), Some(publisher)) = (finished.config_revision, &self.publisher) {
                let revision = gateway_core::routing::ConfigRevision::new(revision.get()).map_err(
                    |error| WorkerTaskError::safe("优选配置版本无效").with_source(error),
                )?;
                publisher.publish_committed(revision).await;
            }
            if !finished.retry_preference {
                break;
            }
            attempts += 1;
        }
        Ok(())
    }

    fn preference_decision(
        &self,
        rule: &EvaluationPreferenceSnapshot,
        claim: &ClaimedEvaluation,
        completion: &EvaluationCompletion,
        exhausted: bool,
    ) -> EvaluationPreferenceDecision {
        let desired_member = if completion.status == EvaluationItemStatus::Succeeded {
            match completion
                .observation
                .turn_state
                .as_ref()
                .map(|probe| probe.outcome)
            {
                Some(TurnStateOutcome::Accepted) => Some(true),
                Some(TurnStateOutcome::Refreshed) => Some(false),
                _ => None,
            }
        } else {
            None
        };
        let mut decision = EvaluationPreferenceDecision {
            rule: rule.clone(),
            desired_member,
            expected_revision: None,
            skip_reason: None,
        };
        if decision.desired_member.is_none() {
            decision.skip_reason = Some("测票未得出明确结果".into());
            return decision;
        }
        if exhausted {
            decision.desired_member = None;
            decision.skip_reason = Some("账号配置持续变化，已保留原分组".into());
            return decision;
        }
        let current = self.snapshot.acquire().ok().and_then(|snapshot| {
            let account = ProviderAccountId::new(&claim.item.account_id).ok()?;
            let key = ClientApiKeyId::new(&claim.parameters.client_key_id).ok()?;
            let policy = snapshot.client_policy(&key)?;
            if policy.authorize().is_err()
                || !policy.account_scope().allows(&account)
                || !super::accounts::independent_preference_key(policy, &rule.group_id)
            {
                return None;
            }
            crate::model::Revision::new(snapshot.revision().get()).ok()
        });
        if current.is_none() {
            decision.desired_member = None;
            decision.skip_reason = Some("检测 Key 的授权范围已变化或配置暂不可用".into());
        }
        decision.expected_revision = current;
        decision
    }

    pub(super) async fn watch_stop(
        &self,
        item_id: &str,
        judgment_id: Option<&str>,
        parent: &CancellationToken,
    ) -> Result<(), AdminStoreError> {
        loop {
            if parent.is_cancelled()
                || self
                    .repository
                    .is_work_stop_requested(item_id, judgment_id)
                    .await?
            {
                return Ok(());
            }
            tokio::select! {
                _ = parent.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    async fn execute(
        &self,
        claim: &ClaimedEvaluation,
        cancellation: CancellationToken,
    ) -> Result<EvaluationCompletion, WorkerTaskError> {
        let started_at = Instant::now();
        let mut completion = empty_completion();
        if self
            .repository
            .is_work_stop_requested(&claim.item.id, None)
            .await
            .map_err(store_error)?
        {
            completion.status = EvaluationItemStatus::Cancelled;
            completion.message = Some("测评已停止或账号不再可用".to_owned());
            return Ok(completion);
        }
        if let Err(error) = claim.parameters.validate() {
            completion.message = Some(error.to_string());
            completion.error_code = Some("evaluation.invalid_parameters".to_owned());
            return Ok(completion);
        }
        let supported_grader = if let Some(definition) = &claim.profile.definition {
            claim.profile.grader_version == DEFINITION_GRADER_VERSION
                && claim.profile.prompt_version == definition.version()
                && claim.profile.prompt == definition.prompt
        } else {
            claim.profile.grader_version == claim.parameters.kind.grader_version()
                && claim.profile.prompt_version == claim.parameters.kind.prompt_version()
        };
        if claim.profile.request_profile_version != claim.parameters.kind.request_profile_version()
            || !supported_grader
        {
            completion.error_code = Some("evaluation.unsupported_profile".to_owned());
            completion.message =
                Some("本版本不支持这条排队任务的实验版本，请重新创建测评".to_owned());
            return Ok(completion);
        }
        if claim.parameters.kind == EvaluationKind::Turnstate {
            return self.execute_turnstate(claim, cancellation).await;
        }
        let request = build_request(claim, cancellation)?;
        let mut started = match self.execution.start_account_execution(request).await {
            Ok(started) => started,
            Err(error) => {
                apply_error(&mut completion, &error);
                completion.duration_ms = elapsed_ms(started_at);
                return Ok(completion);
            }
        };
        let request_id = started.request_id.as_str().to_owned();
        let retained = self
            .repository
            .record_request_id(&claim.item.id, &request_id)
            .await;
        if !matches!(retained, Ok(true)) {
            started.session.cancel();
            started.session.detach_finalize().await;
            return Err(match retained {
                Err(error) => store_error(error),
                _ => WorkerTaskError::safe("测评任务在记录请求标识时已失效"),
            });
        }
        let mut collector = EvaluationCollector::default();
        let collected = started
            .session
            .collect_uncommitted_into(&mut collector)
            .await;
        let complete = collected.is_ok() && collector.complete;
        let assessment = if let Some(definition) = &claim.profile.definition {
            assess_definition(definition, &collector.text, complete)
        } else {
            assess_response(claim.parameters.kind, &collector.text, complete)
        };
        let judge_target = claim.profile.definition.as_ref().and_then(|definition| {
            if !complete
                || (definition.output_format == EvaluationOutputFormat::Html
                    && assessment.artifact.is_none())
            {
                return None;
            }
            match &definition.grader {
                EvaluationGrader::Model { config } => {
                    Some(EvaluationJudgeTarget::Account(config.account_id.clone()))
                }
                EvaluationGrader::Typesafe { .. } => Some(EvaluationJudgeTarget::TypeSafe),
                _ => None,
            }
        });
        let saved = self
            .repository
            .save_output(
                &claim.item.id,
                EvaluationOutput {
                    response_text: collector.text,
                    artifact_html: assessment.artifact,
                },
            )
            .await;
        if !matches!(saved, Ok(true)) {
            started.session.cancel();
            started.session.detach_finalize().await;
            return Err(match saved {
                Err(error) => store_error(error),
                _ => WorkerTaskError::safe("测评任务在保存输出时已失效"),
            });
        }
        let result = match collected {
            Ok(()) => started.session.commit_downstream(Some(200)).await,
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {
                completion.status = EvaluationItemStatus::Succeeded;
                completion.verdict = assessment.verdict;
                completion.final_answer = assessment.final_answer.map(i64::from);
                completion.message = assessment.message.map(str::to_owned);
                completion.judge_target = judge_target;
            }
            Err(EngineError::CollectionLimitExceeded) => {
                completion.error_code = Some("evaluation.output_limit".to_owned());
                completion.message =
                    Some("回答超过 1 MiB，已停止接收；保留的部分不参与判分".to_owned());
            }
            Err(error) => apply_error(&mut completion, &gateway_error_from_engine(&error)),
        }
        if !started.session.is_finalized() {
            started.session.cancel();
            started.session.detach_finalize().await;
        }
        completion.duration_ms = elapsed_ms(started_at);
        if let Some(observation) = self.completed_observation(&request_id).await {
            completion.ttft_ms = observation
                .first_token_ms
                .and_then(|value| i64::try_from(value).ok());
            completion.observation = observation.into();
        }
        Ok(completion)
    }

    pub(super) async fn completed_observation(&self, request_id: &str) -> Option<UsageRecord> {
        // Core 的观测缓冲区异步落库；短暂等待缺失或未终结记录，不推断未知用量
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match self.observability.usage_record_detail(request_id).await {
                    Ok(detail) if detail.request.completed_at.is_some() => {
                        return Some(detail.request);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == AdminStoreErrorKind::NotFound => {}
                    Err(_) => return None,
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .ok()
        .flatten()
    }
}

impl EvaluationRunner {
    async fn execute_turnstate(
        &self,
        claim: &ClaimedEvaluation,
        cancellation: CancellationToken,
    ) -> Result<EvaluationCompletion, WorkerTaskError> {
        let started_at = Instant::now();
        let mut completion = empty_completion();
        let account = ProviderAccountId::new(&claim.item.account_id)
            .map_err(|error| WorkerTaskError::safe("测票账号无效").with_source(error))?;
        let key = ClientApiKeyId::new(&claim.parameters.client_key_id)
            .map_err(|error| WorkerTaskError::safe("测票 Key 无效").with_source(error))?;
        let snapshot = self
            .snapshot
            .acquire()
            .map_err(|error| WorkerTaskError::safe("测票授权配置暂不可用").with_source(error))?;
        if !snapshot.client_policy(&key).is_some_and(|policy| {
            policy.authorize().is_ok()
                && policy.account_scope().allows(&account)
                && claim
                    .profile
                    .account_preference
                    .as_ref()
                    .is_none_or(|rule| {
                        super::accounts::independent_preference_key(policy, &rule.group_id)
                    })
        }) {
            completion.error_code = Some("policy_denied".into());
            completion.message = Some("当前 Key 不再授权此账号，未发送测票请求".into());
            return Ok(completion);
        }
        let Some(providers) = &self.providers else {
            completion.message = Some("测票服务暂不可用".into());
            return Ok(completion);
        };
        let kind = gateway_core::routing::ProviderKind::new("openai")
            .map_err(|error| WorkerTaskError::safe("测票 Provider 无效").with_source(error))?;
        let provider = providers
            .require(&kind)
            .map_err(|error| WorkerTaskError::safe("测票 Provider 不可用").with_source(error))?;
        let model = gateway_core::routing::UpstreamModelId::new(claim.parameters.model.as_str())
            .map_err(|error| WorkerTaskError::safe("测票模型无效").with_source(error))?;
        let configuration = snapshot
            .client_policy(&key)
            .and_then(|policy| policy.account_scope().request_profile(&kind))
            .or_else(|| snapshot.settings().request_profiles().get(&kind));
        let child = cancellation.child_token();
        let result = tokio::time::timeout(
            Duration::from_secs(u64::from(claim.parameters.timeout_seconds)),
            provider.probe_turn_state(&account, &model, configuration, child.clone()),
        )
        .await;
        child.cancel();
        let report = match result {
            Ok(Ok(report)) => report,
            Ok(Err(_)) => TurnStateProbeResult::new(
                Vec::new(),
                Some("测票账号或凭据暂不可用，未得到有效结果".into()),
            ),
            Err(_) => TurnStateProbeResult::new(
                Vec::new(),
                Some("测票总耗时超过设置的超时，未继续发送".into()),
            ),
        };
        completion.status = if cancellation.is_cancelled() {
            EvaluationItemStatus::Cancelled
        } else if report.error.is_some() {
            EvaluationItemStatus::Failed
        } else {
            EvaluationItemStatus::Succeeded
        };
        completion.duration_ms = elapsed_ms(started_at);
        completion.message = Some(report.summary().to_owned());
        if report.error.is_some() {
            completion.error_code = Some("evaluation.turnstate_unavailable".into());
        }
        let output = serde_json::to_string_pretty(&report)
            .map_err(|error| WorkerTaskError::safe("测票结果编码失败").with_source(error))?;
        if !self
            .repository
            .save_output(
                &claim.item.id,
                EvaluationOutput {
                    response_text: output,
                    artifact_html: None,
                },
            )
            .await
            .map_err(store_error)?
        {
            return Err(WorkerTaskError::safe("测票输出保存时任务已失效"));
        }
        completion.observation = EvaluationObservation {
            turn_state: Some(report),
            provider: Some("openai".into()),
            account_id: Some(claim.item.account_id.clone()),
            upstream_model: Some(claim.parameters.model.as_str().to_owned()),
            reasoning_effort: Some("low".into()),
            ..Default::default()
        };
        Ok(completion)
    }
}

fn build_request(
    claim: &ClaimedEvaluation,
    cancellation: CancellationToken,
) -> Result<StartAccountExecution, WorkerTaskError> {
    EvaluationModelCall {
        model: &claim.parameters.model,
        client_key_id: &claim.parameters.client_key_id,
        account_id: &claim.item.account_id,
        reasoning_effort: claim.parameters.reasoning_effort.as_deref(),
        timeout_seconds: claim.parameters.timeout_seconds,
        input: json!([{"type": "message", "role": "user", "content": claim.profile.prompt}]),
        endpoint: "/internal/evaluations",
    }
    .build(cancellation)
}

pub(super) struct EvaluationModelCall<'a> {
    pub model: &'a gateway_core::routing::PublicModelId,
    pub client_key_id: &'a str,
    pub account_id: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub timeout_seconds: u16,
    pub input: serde_json::Value,
    pub endpoint: &'a str,
}

impl EvaluationModelCall<'_> {
    pub fn build(
        self,
        cancellation: CancellationToken,
    ) -> Result<StartAccountExecution, WorkerTaskError> {
        let mut body = json!({
            "model": self.model.as_str(),
            "input": self.input,
            "stream": true,
            "store": false,
            "tools": [],
        })
        .as_object()
        .cloned()
        .ok_or_else(|| WorkerTaskError::safe("测评请求构造失败"))?;
        if let Some(effort) = self.reasoning_effort {
            body.insert("reasoning".to_owned(), json!({"effort": effort}));
        }
        let operation = ProtocolPayload::json_object("openai", body)
            .map_err(|error| WorkerTaskError::safe("测评请求协议无效").with_source(error))?;
        Ok(StartAccountExecution {
            client_key_id: ClientApiKeyId::new(self.client_key_id.to_owned())
                .map_err(|error| WorkerTaskError::safe("测评 Key 标识无效").with_source(error))?,
            account_id: ProviderAccountId::new(self.account_id.to_owned())
                .map_err(|error| WorkerTaskError::safe("测评账号标识无效").with_source(error))?,
            public_model: self.model.clone(),
            operation: Operation::Generate(GenerateRequest::from_protocol_payload(operation)),
            metadata: ExecutionRequestMetadata {
                protocol: "openai".to_owned(),
                endpoint: self.endpoint.to_owned(),
                transport: ClientTransport::InternalAdmin,
                stream: true,
                client_ip: None,
                user_agent: None,
                previous_response_id: None,
            },
            deadline_at: SystemTime::now() + Duration::from_secs(u64::from(self.timeout_seconds)),
            cancellation,
        })
    }
}

fn empty_completion() -> EvaluationCompletion {
    EvaluationCompletion {
        judge_target: None,
        status: EvaluationItemStatus::Failed,
        verdict: EvaluationVerdict::Ungraded,
        final_answer: None,
        error_code: None,
        message: None,
        ttft_ms: None,
        duration_ms: 0,
        completed_at: Utc::now(),
        observation: Default::default(),
        account_preference: None,
    }
}

fn apply_error(completion: &mut EvaluationCompletion, error: &GatewayError) {
    completion.status = if error.kind() == GatewayErrorKind::Cancelled {
        EvaluationItemStatus::Cancelled
    } else {
        EvaluationItemStatus::Failed
    };
    completion.error_code = Some(error.kind().as_str().to_owned());
    completion.message = Some(error.safe_message().to_owned());
}

fn elapsed_ms(started_at: Instant) -> i64 {
    i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX)
}

pub(super) fn store_error(error: AdminStoreError) -> WorkerTaskError {
    WorkerTaskError::safe("测评存储暂时不可用").with_source(error)
}
