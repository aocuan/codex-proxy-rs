//! 模型裁判的结构化结论与无效输出不能混同为答错

use gateway_admin::model::evaluation::{EvaluationVerdict, parse_judgment};

#[test]
fn judge_distinguishes_negative_verdict_unknown_and_invalid_output() {
    for (source, verdict) in [
        (
            r#"{"pass":true,"reason":"答案符合参考"}"#,
            EvaluationVerdict::Correct,
        ),
        (
            r#"{"pass":false,"reason":"推导矛盾"}"#,
            EvaluationVerdict::Wrong,
        ),
        (
            r#"{"pass":null,"reason":"材料不足"}"#,
            EvaluationVerdict::Ungraded,
        ),
    ] {
        assert_eq!(parse_judgment(source).unwrap().0, verdict);
    }
    for invalid in [
        "not json",
        r#"{"reason":"缺少结论字段"}"#,
        r#"{"pass":"false","reason":"类型错误"}"#,
        r#"{"pass":true,"reason":""}"#,
        r#"{"pass":true,"reason":"理由","score":100}"#,
    ] {
        assert!(parse_judgment(invalid).is_err());
    }
}
