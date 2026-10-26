//! 验证测评接口的管理员边界、严格输入与页面配置合同

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::AdminTestFixture;

fn app(fixture: &AdminTestFixture) -> Router {
    crate::openai::api_router_with_admin(fixture.services.clone()).layer(axum::Extension(
        axum::extract::ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 41000))),
    ))
}

fn request(method: Method, uri: &str, body: Value, authenticated: bool) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if authenticated {
        builder = builder.header(header::COOKIE, "cpr_session=valid-session");
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn parameters() -> Value {
    json!({
        "kind": "candy", "clientKeyId": "key_evaluation",
        "accountIds": ["acct_evaluation"], "model": "gpt-5",
        "reasoningEffort": "high", "repetitions": 1, "timeoutSeconds": 600,
    })
}

#[tokio::test]
async fn evaluation_routes_require_admin_before_accessing_history_or_execution() {
    let fixture = AdminTestFixture::new().await;
    for (method, uri) in [
        (Method::GET, "/api/admin/evaluations/config"),
        (Method::POST, "/api/admin/evaluations/accounts/access"),
        (Method::GET, "/api/admin/evaluations/settings"),
        (Method::POST, "/api/admin/evaluations/settings/update"),
        (Method::GET, "/api/admin/evaluations/runs"),
        (
            Method::GET,
            "/api/admin/evaluations/accounts/history?accountIds=acct_one",
        ),
        (Method::POST, "/api/admin/evaluations/items/retry-judgment"),
        (Method::POST, "/api/admin/evaluations/runs/create"),
        (
            Method::GET,
            "/api/admin/evaluations/runs/detail?id=eval_one",
        ),
        (Method::POST, "/api/admin/evaluations/runs/stop"),
        (
            Method::GET,
            "/api/admin/evaluations/items/output?id=eval_item_one",
        ),
        (Method::GET, "/api/admin/evaluations/plans"),
        (Method::POST, "/api/admin/evaluations/plans/create"),
        (Method::POST, "/api/admin/evaluations/plans/update"),
        (Method::POST, "/api/admin/evaluations/plans/delete"),
    ] {
        let response = app(&fixture)
            .oneshot(request(method, uri, Value::Null, false))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn evaluation_config_exposes_frozen_tests_and_server_limits() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    let response = app(&fixture)
        .oneshot(request(
            Method::GET,
            "/api/admin/evaluations/config",
            Value::Null,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["data"]["timezone"], "Asia/Shanghai");
    assert_eq!(body["data"]["tests"][0]["kind"], "candy");
    assert_eq!(body["data"]["tests"][1]["kind"], "pelican");
    assert_eq!(body["data"]["limits"]["accounts"], 20);
    assert_eq!(body["data"]["limits"]["parallelRequests"], 2);
    assert!(
        body["data"]["tests"][0]["prompt"]
            .as_str()
            .unwrap()
            .contains("最终答案")
    );
}

#[tokio::test]
async fn invalid_evaluation_requests_never_reach_the_repository() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    let valid_submit = json!({
        "submissionId": "73fcf201-5bbb-4d60-906f-3b3b9fa67d01",
        "parameters": parameters(),
    });
    let mut unknown_field = valid_submit.clone();
    unknown_field["unrecognizedField"] = json!(true);
    let mut unknown_parameter = valid_submit.clone();
    unknown_parameter["parameters"]["unrecognizedField"] = json!(true);
    let mut invalid_id = valid_submit.clone();
    invalid_id["submissionId"] = json!("not-a-uuid");
    let mut empty_accounts = valid_submit.clone();
    empty_accounts["parameters"]["accountIds"] = json!([]);
    let mut pelican_repetitions = valid_submit;
    pelican_repetitions["parameters"]["kind"] = json!("pelican");
    pelican_repetitions["parameters"]["repetitions"] = json!(2);
    for (body, expected_status) in [
        (unknown_field, StatusCode::UNPROCESSABLE_ENTITY),
        (unknown_parameter, StatusCode::UNPROCESSABLE_ENTITY),
        (invalid_id, StatusCode::BAD_REQUEST),
        (empty_accounts, StatusCode::BAD_REQUEST),
        (pelican_repetitions, StatusCode::BAD_REQUEST),
    ] {
        let response = app(&fixture)
            .oneshot(request(
                Method::POST,
                "/api/admin/evaluations/runs/create",
                body,
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
    }
    let response = app(&fixture)
        .oneshot(request(
            Method::GET,
            "/api/admin/evaluations/runs?limit=0",
            Value::Null,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn plans_reject_invalid_or_ambiguous_intervals_before_storage() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    for (interval, cron, expected) in [
        (json!(0), "", StatusCode::BAD_REQUEST),
        (json!(525601), "", StatusCode::BAD_REQUEST),
        (json!(45), "* * * * *", StatusCode::BAD_REQUEST),
        (Value::Null, "", StatusCode::BAD_REQUEST),
        (json!(1.5), "", StatusCode::UNPROCESSABLE_ENTITY),
        (json!(-1), "", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let response = app(&fixture)
            .oneshot(request(
                Method::POST,
                "/api/admin/evaluations/plans/create",
                json!({
                    "name": "间隔检查", "parameters": parameters(), "enabled": true,
                    "intervalMinutes": interval, "cronExpression": cron,
                }),
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

fn access_snapshot(
    enabled: bool,
    restricted: bool,
    group_enabled: bool,
) -> gateway_core::routing::RuntimeSnapshot {
    use gateway_core::{
        account::{
            ProviderAccountId,
            scope::{
                AccountGroupId, ClientRoutingScope, FrozenAccountScope, RoutingGroupSnapshot,
                RuntimeAccount, RuntimeAccountDirectory,
            },
        },
        identity::ProviderKind,
        policy::{ClientApiKeyId, ClientPolicy, PlaintextClientApiKey},
        routing::{ConfigRevision, RuntimeSnapshot},
        settings::SettingsValues,
    };
    use std::sync::Arc;
    let provider = ProviderKind::new("openai").unwrap();
    let group = AccountGroupId::new(format!("grp_{}", "a".repeat(32))).unwrap();
    let directory = Arc::new(RuntimeAccountDirectory::new(
        [
            (
                ProviderAccountId::new("acct_evaluation").unwrap(),
                RuntimeAccount::new(provider.clone(), [group.clone()].into()),
            ),
            (
                ProviderAccountId::new("acct_business").unwrap(),
                RuntimeAccount::new(provider.clone(), Default::default()),
            ),
        ]
        .into(),
    ));
    let scope = if restricted {
        ClientRoutingScope::restricted(
            vec![RoutingGroupSnapshot::new(group.clone(), "生产账号".into())],
            if group_enabled {
                [group].into()
            } else {
                Default::default()
            },
            if group_enabled {
                [provider.clone()].into()
            } else {
                Default::default()
            },
        )
        .unwrap()
    } else {
        ClientRoutingScope::all_accounts()
    };
    let policy = ClientPolicy::new(
        ClientApiKeyId::new("key_evaluation").unwrap(),
        PlaintextClientApiKey::new("test-only-key").unwrap(),
        Arc::new(FrozenAccountScope::new(directory, scope)),
        enabled,
        Default::default(),
    )
    .with_name_snapshot(Some("质量检测".into()));
    RuntimeSnapshot::new(
        ConfigRevision::new(1).unwrap(),
        SettingsValues::new(1, "smart", Default::default(), None, None),
        vec![provider],
        vec![],
        vec![policy],
    )
    .unwrap()
}

#[tokio::test]
async fn account_access_uses_the_current_key_scope_and_fails_closed() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    for (enabled, restricted, group_enabled, expected) in [
        (true, true, true, [true, false, false]),
        (true, false, true, [true, true, false]),
        (true, true, false, [false, false, false]),
        (false, false, true, [false, false, false]),
    ] {
        fixture
            .published_snapshot
            .publish(access_snapshot(enabled, restricted, group_enabled));
        let response = app(&fixture).oneshot(request(Method::POST, "/api/admin/evaluations/accounts/access", json!({
            "clientKeyId": "key_evaluation", "accountIds": ["acct_evaluation", "acct_business", "acct_deleted"],
        }), true)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        let values = body["data"].as_array().unwrap();
        for (value, allowed) in values.iter().zip(expected) {
            assert_eq!(value["allowed"], allowed);
            assert_eq!(value["reason"].is_null(), allowed);
        }
    }
    let response = app(&fixture)
        .oneshot(request(
            Method::POST,
            "/api/admin/evaluations/accounts/access",
            json!({
                "clientKeyId": "key_deleted", "accountIds": ["acct_evaluation"],
            }),
            true,
        ))
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["data"][0]["allowed"], false);
    assert!(
        body["data"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("已不存在")
    );
    fixture.published_snapshot.suspend();
    let response = app(&fixture)
        .oneshot(request(
            Method::POST,
            "/api/admin/evaluations/accounts/access",
            json!({
                "clientKeyId": "key_evaluation", "accountIds": ["acct_evaluation"],
            }),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn out_of_scope_evaluation_and_judge_are_rejected_before_enqueue() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-session");
    fixture
        .published_snapshot
        .publish(access_snapshot(true, true, true));
    let mut outside = parameters();
    outside["accountIds"] = json!(["acct_business"]);
    let mut judge = parameters();
    judge["definition"] = json!({
        "name": "判题授权", "prompt": "回答 42", "outputFormat": "text",
        "grader": {"type": "model", "config": {
            "clientKeyId": "key_evaluation", "accountId": "acct_business", "model": "gpt-5",
            "reasoningEffort": null, "timeoutSeconds": 60, "rubric": "答案为 42", "referenceAnswer": "42",
        }},
    });
    for params in [outside, judge] {
        for (uri, payload) in [
            (
                "/api/admin/evaluations/runs/create",
                json!({"submissionId": "73fcf201-5bbb-4d60-906f-3b3b9fa67d01", "parameters": params}),
            ),
            (
                "/api/admin/evaluations/plans/create",
                json!({"name": "授权检查", "enabled": true, "intervalMinutes": 60, "parameters": params}),
            ),
        ] {
            let response = app(&fixture)
                .oneshot(request(Method::POST, uri, payload, true))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                    .unwrap();
            assert!(
                body["message"].as_str().unwrap().contains("未授权"),
                "{body}"
            );
        }
    }
}
