//! OpenAI 两请求测票传输，票据与 Cookie 仅在本次会话内存中存在

use std::{collections::BTreeMap, time::Instant};

use futures::StreamExt as _;
use gateway_admin::{
    model::evaluation::{
        TurnStateDiagnostic, TurnStateProbeResult, TurnStateStep, TurnStateUnavailable,
    },
    ports::provider::{ProviderAdminError, ProviderAdminErrorKind},
};
use gateway_core::{
    account::{CredentialState, ProviderAccountId, ProviderAccountStore, UpstreamConnection},
    lifecycle::CancellationToken,
    routing::UpstreamModelId,
};
use gateway_protocol::openai::sse::SseEventDecoder;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{
    client::build_account_http_client,
    diagnostics::{CodexFailureCategory, classify_probe_failure},
    profile::CodexWireProfile,
};
use crate::credential::CodexCredentialCodec;

const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const MAX_BODY_BYTES: usize = 256 * 1024;

pub(crate) async fn probe(
    accounts: &dyn ProviderAccountStore,
    profile: &CodexWireProfile,
    id: &ProviderAccountId,
    model: &UpstreamModelId,
    cancellation: CancellationToken,
) -> Result<TurnStateProbeResult, ProviderAdminError> {
    let loaded = accounts
        .load_current_credential(id)
        .await
        .map_err(|_| unavailable())?;
    let connection = UpstreamConnection::for_account(&loaded.account);
    let mut result =
        probe_connected(accounts, profile, id, model, cancellation, &connection).await?;
    result.connection_revision = Some(connection.revision.clone());
    let current = accounts
        .load_current_credential(id)
        .await
        .map_err(|_| unavailable())?;
    if UpstreamConnection::for_account(&current.account) != connection {
        result.outcome = gateway_admin::model::evaluation::TurnStateOutcome::Unknown;
        result.error = Some("测票期间凭据或代理发生变化，结果仅保留为历史".into());
    }
    Ok(result)
}

async fn probe_connected(
    accounts: &dyn ProviderAccountStore,
    profile: &CodexWireProfile,
    id: &ProviderAccountId,
    model: &UpstreamModelId,
    cancellation: CancellationToken,
    connection: &UpstreamConnection,
) -> Result<TurnStateProbeResult, ProviderAdminError> {
    let mut steps = Vec::new();
    let mut ticket = String::new();
    let mut cookies = BTreeMap::new();
    let session = uuid::Uuid::new_v4().to_string();
    for _ in 0..2 {
        let loaded = accounts
            .load_current_credential(id)
            .await
            .map_err(|_| unavailable())?;
        let account = &loaded.account;
        if !account.enabled()
            || account.provider().as_str() != "openai"
            || account.authentication_kind() != "oauth"
            || matches!(
                account.credential_state(),
                CredentialState::Expired | CredentialState::Banned | CredentialState::Invalid
            )
        {
            return Ok(TurnStateProbeResult::new(
                steps,
                Some("账号不存在、已停用或不是可用的 OpenAI OAuth 账号".into()),
            ));
        }
        if UpstreamConnection::for_account(account) != *connection {
            return Ok(TurnStateProbeResult::new(
                steps,
                Some("两次请求之间凭据或代理发生变化，请重新测票".into()),
            ));
        }
        if account.access_token_expires_at().is_some_and(|at| {
            at <= std::time::SystemTime::now() + std::time::Duration::from_secs(30)
        }) {
            return Ok(TurnStateProbeResult::new(
                steps,
                Some("访问令牌即将过期，等待正常刷新后再测票".into()),
            ));
        }
        let credential =
            CodexCredentialCodec::decode(&loaded.credential).map_err(|_| unavailable())?;
        let Some(secret) = credential.authentication.oauth() else {
            return Err(unavailable());
        };
        let Some(account_id) = account.upstream_account_id() else {
            return Err(unavailable());
        };
        let client = build_account_http_client(id.as_str(), account.outbound_proxy())
            .map_err(|_| unavailable())?;
        let cookie = cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        let before = route(&cookies);
        let mut request = client.post(ENDPOINT)
            .bearer_auth(secret.access_token.expose_secret())
            .header("chatgpt-account-id", account_id)
            .header("user-agent", profile.user_agent())
            .header("originator", &profile.originator)
            .header("version", &profile.codex_version)
            .header("session_id", &session)
            .header("openai-beta", "responses=experimental")
            .header("accept", "text/event-stream")
            .json(&json!({"model":model.as_str(),"instructions":"You are Codex, a coding agent.","input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}],"reasoning":{"effort":"low"},"stream":true,"store":false}));
        if !ticket.is_empty() {
            request = request.header("x-codex-turn-state", &ticket);
        }
        if !cookie.is_empty() {
            request = request.header("cookie", cookie);
        }
        let start = Instant::now();
        // 取消后丢弃本次网络 Future，reqwest 正文及连接随作用域释放，不重放可能已收费的请求
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err("测票已取消"),
            result = exchange(request, &mut cookies) => result,
        };
        match result {
            Ok((status, got, completed, diagnostic)) => {
                steps.push(TurnStateStep {
                    status,
                    duration_ms: millis(start),
                    ticket_length: got.len(),
                    ticket_fingerprint: fingerprint(&got),
                    route_changed: !before.is_empty() && before != route(&cookies),
                    completed,
                });
                if !completed {
                    let mut report = TurnStateProbeResult::new(
                        steps,
                        Some(
                            if status == 200 {
                                "上游响应未完整结束，测票未判定"
                            } else {
                                "上游拒绝测票请求，未继续发送"
                            }
                            .into(),
                        ),
                    );
                    report.diagnostic = diagnostic;
                    return Ok(report);
                }
                ticket = got;
                if ticket.is_empty() && steps.len() == 1 {
                    break;
                }
            }
            Err(error) => return Ok(TurnStateProbeResult::new(steps, Some(error.into()))),
        }
    }
    Ok(TurnStateProbeResult::new(steps, None))
}

async fn exchange(
    request: reqwest::RequestBuilder,
    cookies: &mut BTreeMap<String, String>,
) -> Result<(u16, String, bool, Option<TurnStateDiagnostic>), &'static str> {
    let response = request.send().await.map_err(|_| "测票连接失败或超时")?;
    let status = response.status().as_u16();
    let mut ticket = response
        .headers()
        .get("x-codex-turn-state")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    for value in response.headers().get_all("set-cookie") {
        let Some(cookie) = value
            .to_str()
            .ok()
            .and_then(|s| cookie::Cookie::parse(s.to_owned()).ok())
        else {
            continue;
        };
        if cookies.len() < 128 || cookies.contains_key(cookie.name()) {
            if cookie.value().is_empty()
                || cookie.max_age().is_some_and(|age| age.whole_seconds() <= 0)
            {
                cookies.remove(cookie.name());
            } else {
                cookies.insert(cookie.name().to_owned(), cookie.value().to_owned());
            }
        }
    }
    if status != 200 {
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            if body.len() + chunk.len() > 8192 {
                break;
            }
            body.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
        return Ok((
            status,
            ticket,
            false,
            Some(diagnostic(status, "request", &value)),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut decoder = SseEventDecoder::default();
    let mut bytes = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "测票响应读取失败")?;
        bytes += chunk.len();
        if bytes > MAX_BODY_BYTES {
            return Err("测票响应超过 256 KiB");
        }
        for event in decoder.push(&chunk).map_err(|_| "测票 SSE 格式无效")? {
            if event.data == "[DONE]" {
                continue;
            }
            let value: Value = serde_json::from_str(&event.data).map_err(|_| "测票事件格式无效")?;
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .or(event.event.as_deref())
                .unwrap_or_default();
            if kind == "response.metadata"
                && let Some(state) = value
                    .pointer("/headers/x-codex-turn-state")
                    .and_then(Value::as_str)
            {
                ticket = state.to_owned();
            }
            if kind == "response.failed" || kind == "error" || kind == "response.incomplete" {
                return Ok((
                    status,
                    ticket,
                    false,
                    Some(diagnostic(status, "stream", &value)),
                ));
            }
            if kind == "response.completed" {
                let completed = value
                    .pointer("/response/status")
                    .and_then(Value::as_str)
                    .is_none_or(|s| s == "completed");
                return Ok((status, ticket, completed, None));
            }
        }
    }
    Ok((status, ticket, false, None))
}

fn diagnostic(status: u16, stage: &str, value: &Value) -> TurnStateDiagnostic {
    // 仅保存有界机器码，不持久化可能包含账号材料的上游正文
    let code = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() <= 128
                    && !value.is_empty()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
            })
            .map(str::to_owned)
    };
    let error = value
        .get("error")
        .or_else(|| value.pointer("/response/error"))
        .unwrap_or(value);
    let error_code = code(error.get("code"));
    let error_type = code(error.get("type"));
    let status_code = (stage == "request")
        .then_some(status)
        .and_then(|status| reqwest::StatusCode::from_u16(status).ok());
    let unavailable =
        match classify_probe_failure(status_code, error_code.as_deref(), error_type.as_deref()) {
            CodexFailureCategory::CredentialExpired
            | CodexFailureCategory::Banned
            | CodexFailureCategory::IdentityVerificationRequired => {
                Some(TurnStateUnavailable::Authorization)
            }
            CodexFailureCategory::QuotaExhausted | CodexFailureCategory::UsageLimitExhausted => {
                Some(TurnStateUnavailable::Quota)
            }
            CodexFailureCategory::ModelUnsupported => Some(TurnStateUnavailable::Model),
            _ => None,
        };
    TurnStateDiagnostic {
        stage: stage.into(),
        http_status: status,
        error_code: error_code
            .or_else(|| code(value.pointer("/response/incomplete_details/reason"))),
        error_type,
        unavailable,
    }
}

fn route(cookies: &BTreeMap<String, String>) -> Vec<(String, String)> {
    ["__cflb", "__oailb"]
        .into_iter()
        .filter_map(|key| {
            cookies
                .get(key)
                .map(|value| (key.to_owned(), value.clone()))
        })
        .collect()
}
fn fingerprint(ticket: &str) -> String {
    if ticket.is_empty() {
        String::new()
    } else {
        hex::encode(Sha256::digest(ticket.as_bytes()))
    }
}
fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}
fn unavailable() -> ProviderAdminError {
    ProviderAdminError::new(ProviderAdminErrorKind::Unavailable)
}
