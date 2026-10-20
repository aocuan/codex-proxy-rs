//! 测评默认参数与独立判题连接，保存后只用于新建草稿

use gateway_core::{policy::ClientApiKeyId, routing::PublicModelId};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::model::AdminError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationDefaults {
    pub client_key_id: Option<String>,
    pub model: Option<PublicModelId>,
    pub reasoning_effort: Option<String>,
    pub repetitions: u8,
    pub timeout_seconds: u16,
    pub judge: Option<EvaluationJudgeDefaults>,
}

impl Default for EvaluationDefaults {
    fn default() -> Self {
        Self {
            client_key_id: None,
            model: None,
            reasoning_effort: Some("high".to_owned()),
            repetitions: 1,
            timeout_seconds: 600,
            judge: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EvaluationJudgeDefaults {
    Model {
        client_key_id: String,
        account_id: String,
        model: PublicModelId,
        reasoning_effort: Option<String>,
        timeout_seconds: u16,
    },
    Typesafe {
        model: String,
        decision_threshold: u8,
        timeout_seconds: u16,
    },
}

impl EvaluationDefaults {
    pub fn validate(&self) -> Result<(), AdminError> {
        if let Some(id) = &self.client_key_id {
            ClientApiKeyId::new(id.clone()).map_err(|_| AdminError::invalid("计费 Key ID 无效"))?;
        }
        super::definition::validate_effort(self.reasoning_effort.as_deref())?;
        if !(1..=super::MAX_REPETITIONS).contains(&self.repetitions)
            || !(1..=600).contains(&self.timeout_seconds)
        {
            return Err(AdminError::invalid("测评次数或超时无效"));
        }
        match &self.judge {
            Some(EvaluationJudgeDefaults::Model {
                client_key_id,
                account_id,
                reasoning_effort,
                timeout_seconds,
                ..
            }) => {
                super::definition::validate_judge_connection(
                    client_key_id,
                    account_id,
                    reasoning_effort.as_deref(),
                    *timeout_seconds,
                )?;
            }
            Some(EvaluationJudgeDefaults::Typesafe {
                model,
                decision_threshold,
                timeout_seconds,
            }) => {
                super::typesafe::validate_connection(model, *decision_threshold, *timeout_seconds)?;
            }
            None => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationSettings {
    pub revision: i64,
    pub defaults: EvaluationDefaults,
    pub typesafe_configured: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateEvaluationSettings {
    pub revision: i64,
    pub defaults: EvaluationDefaults,
    /// 缺省保留，空字符串清除；明文只进入存储，不进入响应与审计
    pub typesafe_api_key: Option<SecretString>,
}

impl UpdateEvaluationSettings {
    pub fn validate(&self) -> Result<(), AdminError> {
        if self.revision < 0 {
            return Err(AdminError::invalid("测评设置版本无效"));
        }
        self.defaults.validate()?;
        if self.typesafe_api_key.as_ref().is_some_and(|value| {
            let key = value.expose_secret();
            key.len() > 4096
                || key.chars().any(char::is_whitespace)
                || key.chars().any(char::is_control)
        }) {
            return Err(AdminError::invalid("TypeSafe API Key 格式无效"));
        }
        Ok(())
    }
}
