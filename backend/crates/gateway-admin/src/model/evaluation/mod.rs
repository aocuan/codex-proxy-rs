//! 测评预设、自定义规则与实验快照，不执行上游调用或解释凭据

mod benchmark;
mod definition;
mod judgment;
mod records;
mod settings;
mod typesafe;
pub use benchmark::{EvaluationAssessment, assess_definition, assess_response};
pub use definition::*;
pub use judgment::*;
pub use records::*;
pub use settings::*;
pub use typesafe::*;

pub const REQUEST_PROFILE_VERSION: &str = "responses-no-tools-v1";
pub const GRADER_VERSION: &str = "final-answer-v1";
pub const MAX_ARTIFACT_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_ACCOUNTS_PER_RUN: usize = 20;
pub const MAX_REPETITIONS: u8 = 5;
pub const MAX_QUEUED_ITEMS: i64 = 100;
pub const MAX_PARALLEL_REQUESTS: i64 = 2;
pub const MAX_PLAN_INTERVAL_MINUTES: u32 = 525_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationKind {
    Candy,
    Pelican,
}

impl EvaluationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candy => "candy",
            Self::Pelican => "pelican",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Candy => "糖果题",
            Self::Pelican => "画鹈鹕",
        }
    }

    #[must_use]
    pub const fn prompt_version(self) -> &'static str {
        match self {
            Self::Candy => "candy-v1",
            Self::Pelican => "pelican-css-v1",
        }
    }

    #[must_use]
    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Candy => {
                "不使用任何外部工具回答以下问题：\n\n\
在一个黑色的袋子里放有三种口味的糖果，每种糖果有两种不同的形状：圆形和五角星形。\
形状靠手感可以分辨，你可以选择取出的形状，但不能选择口味。数量如下：\n\n\
形状        苹果味  桃子味  西瓜味\n\
圆形           7      9      8\n\
五角星形       7      6      4\n\n\
参赛者需要在活动前决定最多取出的糖果数目。最少取出多少个糖果，\
才能保证手中同时拥有不同形状的苹果味和桃子味的糖？\
圆形苹果味与五角星桃子味，或者圆形桃子味与五角星苹果味，均满足要求。\n\n\
简要解释理由，最后单独一行严格使用“最终答案：N”给出最少数量，N 为整数"
            }
            Self::Pelican => {
                "不使用任何工具，不创建或修改文件，直接在回复中完成任务。\n\n\
创建一个自包含的 HTML 页面，用内联 SVG 绘制鹈鹕骑自行车的 2D 动画。\
动画使用 CSS，不使用 JavaScript、外部素材、网络资源或外部字体。\
展现鹈鹕、自行车和连贯的骑行动作，画面应完整可见。\n\n\
只在一个 ```html 代码块中给出完整 HTML，包含 <!doctype html>、\
<html>、<head>、<body> 和内联 SVG，不需要测试或额外解释"
            }
        }
    }
}

/// 题目结论独立于请求执行状态，网络失败不能变成答错
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationVerdict {
    Correct,
    Wrong,
    Ungraded,
    NotApplicable,
}

impl EvaluationVerdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Correct => "correct",
            Self::Wrong => "wrong",
            Self::Ungraded => "ungraded",
            Self::NotApplicable => "not_applicable",
        }
    }
}
