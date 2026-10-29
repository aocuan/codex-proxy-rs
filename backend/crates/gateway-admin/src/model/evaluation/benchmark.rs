//! 从完整回复提取最终答案或作品正文，原文不作为日志或管理端 HTML 执行

use super::{
    EvaluationAnswerSource, EvaluationDefinition, EvaluationGrader, EvaluationKind,
    EvaluationOutputFormat, EvaluationVerdict, MAX_ARTIFACT_BYTES, MAX_RESPONSE_BYTES,
};

pub struct EvaluationAssessment {
    pub verdict: EvaluationVerdict,
    pub final_answer: Option<u32>,
    pub artifact: Option<String>,
    pub message: Option<&'static str>,
}

/// 仅完整回复可以判分，输出上限与异常结束不能碰巧得到正确答案
#[must_use]
pub fn assess_response(
    kind: EvaluationKind,
    response: &str,
    complete: bool,
) -> EvaluationAssessment {
    if !complete || response.len() > MAX_RESPONSE_BYTES {
        return ungraded("回答未完整接收或超过大小限制");
    }
    match kind {
        EvaluationKind::Turnstate => ungraded("测票只报告路由信号，不参与能力题判分"),
        EvaluationKind::Candy => assess_candy(response),
        EvaluationKind::Pelican => assess_pelican(response),
    }
}

fn ungraded(message: &'static str) -> EvaluationAssessment {
    EvaluationAssessment {
        verdict: EvaluationVerdict::Ungraded,
        final_answer: None,
        artifact: None,
        message: Some(message),
    }
}

fn assess_candy(response: &str) -> EvaluationAssessment {
    let answer = match final_integer(response) {
        Ok(answer) => answer,
        Err(message) => return ungraded(message),
    };
    EvaluationAssessment {
        verdict: if answer == 21 {
            EvaluationVerdict::Correct
        } else {
            EvaluationVerdict::Wrong
        },
        final_answer: Some(answer),
        artifact: None,
        message: None,
    }
}

fn final_integer(response: &str) -> Result<u32, &'static str> {
    // 只接受唯一的末行结论，正文中的数字或相互矛盾的多个答案不参与匹配
    let mut markers = response
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("最终答案"));
    let Some(line) = markers.next() else {
        return Err("回答缺少明确的最终答案");
    };
    if markers.next().is_some() || response.trim_end().lines().last().map(str::trim) != Some(line) {
        return Err("回答包含多个结论或最终答案格式不符合要求");
    }
    let Some(number) = line
        .strip_prefix("最终答案：")
        .or_else(|| line.strip_prefix("最终答案:"))
        .map(str::trim)
        .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
    else {
        return Err("最终答案格式不符合要求");
    };
    let Ok(answer) = number.parse::<u32>() else {
        return Err("最终答案数值超出有效范围");
    };
    Ok(answer)
}

fn assess_pelican(response: &str) -> EvaluationAssessment {
    extract_html(response, true)
}

fn extract_html(response: &str, require_svg: bool) -> EvaluationAssessment {
    let source = response.trim();
    let markup = if source.starts_with("```") {
        let Some((header, rest)) = source.split_once('\n') else {
            return ungraded("回答中没有完整 HTML 代码块");
        };
        if !header.trim().eq_ignore_ascii_case("```html") {
            return ungraded("回答需要使用 HTML 代码块");
        }
        let Some(content) = rest.trim_end().strip_suffix("```") else {
            return ungraded("HTML 代码块未完整结束");
        };
        if content.contains("```") {
            return ungraded("回答包含多个代码块");
        }
        content.trim()
    } else {
        source
    };
    if markup.len() > MAX_ARTIFACT_BYTES {
        return ungraded("作品超过大小限制");
    }
    let lower = markup.to_ascii_lowercase();
    if !(lower.starts_with("<!doctype html") || lower.starts_with("<html"))
        || !lower.ends_with("</html>")
        || (require_svg && (!lower.contains("<svg") || !lower.contains("</svg>")))
    {
        return ungraded("回答中没有完整的 HTML 和 SVG 作品");
    }
    // 提取成功仅表示取得作品，浏览器仍须清洗并在无权限沙箱中预览
    EvaluationAssessment {
        verdict: EvaluationVerdict::NotApplicable,
        final_answer: None,
        artifact: Some(markup.to_owned()),
        message: None,
    }
}

#[must_use]
pub fn assess_definition(
    definition: &EvaluationDefinition,
    response: &str,
    complete: bool,
) -> EvaluationAssessment {
    if !complete || response.len() > MAX_RESPONSE_BYTES {
        return ungraded("回答未完整接收或超过大小限制");
    }
    let mut assessment = if definition.output_format == EvaluationOutputFormat::Html {
        extract_html(response, false)
    } else {
        EvaluationAssessment {
            verdict: EvaluationVerdict::NotApplicable,
            final_answer: None,
            artifact: None,
            message: None,
        }
    };
    if assessment.verdict == EvaluationVerdict::Ungraded {
        return assessment;
    }
    let passed = match &definition.grader {
        EvaluationGrader::None => return assessment,
        EvaluationGrader::Model { .. } | EvaluationGrader::Typesafe { .. } => {
            assessment.verdict = EvaluationVerdict::Ungraded;
            assessment.message = Some("等待 AI 判题");
            return assessment;
        }
        EvaluationGrader::Exact { source, expected } => match source {
            EvaluationAnswerSource::FullText => response.trim() == expected.trim(),
            EvaluationAnswerSource::FinalInteger => match final_integer(response) {
                Ok(value) => {
                    assessment.final_answer = Some(value);
                    expected.parse::<u32>().ok() == Some(value)
                }
                Err(message) => {
                    assessment.verdict = EvaluationVerdict::Ungraded;
                    assessment.message = Some(message);
                    return assessment;
                }
            },
        },
        EvaluationGrader::Regex { pattern } => match super::definition::compile_pattern(pattern) {
            Ok(pattern) => pattern.is_match(response),
            Err(_) => return ungraded("判分正则无效"),
        },
    };
    assessment.verdict = if passed {
        EvaluationVerdict::Correct
    } else {
        EvaluationVerdict::Wrong
    };
    assessment.message = Some(if passed {
        "回答满足判分规则"
    } else {
        "回答未满足判分规则"
    });
    assessment
}
