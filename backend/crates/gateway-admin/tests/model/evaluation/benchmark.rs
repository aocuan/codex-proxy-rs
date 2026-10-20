//! 糖果判分与鹈鹕作品提取的歧义、截断和大小边界测试

use gateway_admin::model::evaluation::{
    EvaluationKind, EvaluationVerdict, MAX_ARTIFACT_BYTES, assess_response,
};

#[test]
fn candy_grades_only_the_explicit_final_answer() {
    for (response, expected, answer) in [
        (
            "9 + 12 = 21\n最终答案：21",
            EvaluationVerdict::Correct,
            Some(21),
        ),
        (
            "21 不满足条件，应该取 29 颗\n最终答案：29",
            EvaluationVerdict::Wrong,
            Some(29),
        ),
        ("最终答案: 21\n\n", EvaluationVerdict::Correct, Some(21)),
    ] {
        let assessment = assess_response(EvaluationKind::Candy, response, true);
        assert_eq!(
            (assessment.verdict, assessment.final_answer),
            (expected, answer)
        );
    }
}

#[test]
fn candy_keeps_ambiguous_or_missing_conclusions_ungraded() {
    for response in [
        "21",
        "最终答案：21\n最终答案：29",
        "最终答案：21 或 29",
        "最终答案：21\n实际上不是",
        "正文说最终答案：21",
        "最终答案：2.1",
        "最终答案：",
        "最终答案：99999999999999999999999999999",
    ] {
        let assessment = assess_response(EvaluationKind::Candy, response, true);
        assert_eq!(
            assessment.verdict,
            EvaluationVerdict::Ungraded,
            "{response}"
        );
        assert_eq!(assessment.final_answer, None);
    }
}

#[test]
fn incomplete_output_cannot_pass_even_with_a_correct_number() {
    let assessment = assess_response(EvaluationKind::Candy, "最终答案：21", false);
    assert_eq!(assessment.verdict, EvaluationVerdict::Ungraded);
}

#[test]
fn pelican_preserves_original_markup_without_claiming_visual_quality() {
    let markup = "<!doctype html><html><body><svg><path d=\"M0 0\"/></svg></body></html>";
    for response in [markup.to_owned(), format!("```html\n{markup}\n```")] {
        let assessment = assess_response(EvaluationKind::Pelican, &response, true);
        assert_eq!(assessment.verdict, EvaluationVerdict::NotApplicable);
        assert_eq!(assessment.artifact.as_deref(), Some(markup));
    }
}

#[test]
fn pelican_rejects_incomplete_ambiguous_or_oversized_artifacts() {
    let huge = format!("<html><svg>{}</svg></html>", " ".repeat(MAX_ARTIFACT_BYTES));
    for response in [
        "```html\n<html><svg></svg></html>",
        "```html\n<html><svg></svg></html>\n```\n```html\n<html></html>\n```",
        "<html><body>没有 SVG</body></html>",
        "<html><svg>",
        &huge,
    ] {
        let assessment = assess_response(EvaluationKind::Pelican, response, true);
        assert_eq!(assessment.verdict, EvaluationVerdict::Ungraded);
        assert!(assessment.artifact.is_none());
    }
}
