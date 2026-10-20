//! 自定义规则、版本隔离与独立判题配置的行为约束

use gateway_admin::model::evaluation::*;
use gateway_core::routing::PublicModelId;

#[test]
fn custom_answers_are_graded_by_the_saved_rule_not_the_preset_answer() {
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.prompt = "计算 6 × 7，末行输出最终答案：N".to_owned();
    definition.grader = EvaluationGrader::Exact {
        source: EvaluationAnswerSource::FinalInteger,
        expected: "42".to_owned(),
    };
    assert_eq!(
        assess_definition(&definition, "最终答案：42", true).verdict,
        EvaluationVerdict::Correct
    );
    assert_eq!(
        assess_definition(&definition, "最终答案：21", true).verdict,
        EvaluationVerdict::Wrong
    );
    assert_eq!(
        assess_definition(&definition, "正文有42", true).verdict,
        EvaluationVerdict::Ungraded
    );
    assert_eq!(
        assess_definition(&definition, "最终答案：42", false).verdict,
        EvaluationVerdict::Ungraded
    );
    definition.grader = EvaluationGrader::Exact {
        source: EvaluationAnswerSource::FullText,
        expected: "hello".to_owned(),
    };
    assert_eq!(
        assess_definition(&definition, " hello\n", true).verdict,
        EvaluationVerdict::Correct
    );
    assert_eq!(
        assess_definition(&definition, "Hello", true).verdict,
        EvaluationVerdict::Wrong
    );
    definition.grader = EvaluationGrader::Regex {
        pattern: r"\Aanswer: [0-9]+\z".to_owned(),
    };
    assert!(definition.validate(&[]).is_ok());
    assert_eq!(
        assess_definition(&definition, "answer: 42", true).verdict,
        EvaluationVerdict::Correct
    );
    assert_eq!(
        assess_definition(&definition, "wrong answer: 42", true).verdict,
        EvaluationVerdict::Wrong
    );
}

#[test]
fn html_without_a_grader_preserves_generic_artifacts_without_claiming_quality() {
    let definition = EvaluationDefinition::preset(EvaluationKind::Pelican);
    let html = "<!doctype html><html><body><h1>作品</h1></body></html>";
    let result = assess_definition(&definition, html, true);
    assert_eq!(result.verdict, EvaluationVerdict::NotApplicable);
    assert_eq!(result.artifact.as_deref(), Some(html));
    assert_eq!(
        assess_definition(&definition, "<html>partial", true).verdict,
        EvaluationVerdict::Ungraded
    );
}

#[test]
fn definition_versions_ignore_names_but_include_questions_and_rules() {
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    let original = definition.version();
    definition.name = "新的显示名称".to_owned();
    assert_eq!(definition.version(), original);
    definition.prompt.push_str("\n说明推导");
    assert_ne!(definition.version(), original);
    let changed_question = definition.version();
    definition.grader = EvaluationGrader::None;
    assert_ne!(definition.version(), changed_question);
}

#[test]
fn grader_configuration_rejects_self_judging_and_invalid_patterns() {
    let mut definition = EvaluationDefinition::preset(EvaluationKind::Candy);
    definition.grader = EvaluationGrader::Model {
        config: EvaluationJudgeConfig {
            client_key_id: "key_judge".to_owned(),
            account_id: "acct_judge".to_owned(),
            model: PublicModelId::new("test-model").unwrap(),
            reasoning_effort: None,
            timeout_seconds: 180,
            rubric: "答案正确且推导完整".to_owned(),
            reference_answer: None,
        },
    };
    assert!(definition.validate(&["acct_target".to_owned()]).is_ok());
    assert!(definition.validate(&["acct_judge".to_owned()]).is_err());
    definition.grader = EvaluationGrader::Regex {
        pattern: "(".to_owned(),
    };
    assert!(definition.validate(&[]).is_err());
    definition.grader = EvaluationGrader::Exact {
        source: EvaluationAnswerSource::FullText,
        expected: " ".to_owned(),
    };
    assert!(definition.validate(&[]).is_err());
}
