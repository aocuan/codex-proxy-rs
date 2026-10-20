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
        let cancellation = parent.child_token();
        let execution = self.execute(&claim, cancellation.clone());
        tokio::pin!(execution);
        let stop = self.watch_stop(&claim.item.run_id, &parent);
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
        self.repository
            .finish_item(&claim.item.id, result)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    pub(super) async fn watch_stop(
        &self,
        run_id: &str,
        parent: &CancellationToken,
    ) -> Result<(), AdminStoreError> {
        loop {
            if parent.is_cancelled() || self.repository.is_stop_requested(run_id).await? {
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
            .is_stop_requested(&claim.item.run_id)
            .await
            .map_err(store_error)?
        {
            completion.status = EvaluationItemStatus::Cancelled;
            completion.message = Some("管理员已停止本轮测评".to_owned());
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
            claim.profile.grader_version == GRADER_VERSION
                && claim.profile.prompt_version == claim.parameters.kind.prompt_version()
        };
        if claim.profile.request_profile_version != REQUEST_PROFILE_VERSION || !supported_grader {
            completion.error_code = Some("evaluation.unsupported_profile".to_owned());
            completion.message =
                Some("本版本不支持这条排队任务的实验版本，请重新创建测评".to_owned());
            return Ok(completion);
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
