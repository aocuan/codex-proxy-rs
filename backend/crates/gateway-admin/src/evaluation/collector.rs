//! 只保留回答正文与完整性事实，不把 wire、凭据或隐藏推理收进作品

use gateway_core::{
    engine::execution::{CollectionLimitExceeded, UncommittedEventCollector},
    event::{FinishReason, GatewayEvent, ProviderEvent},
};

use crate::model::evaluation::MAX_RESPONSE_BYTES;

pub(super) struct EvaluationCollector {
    pub text: String,
    pub complete: bool,
    limit: usize,
}

impl Default for EvaluationCollector {
    fn default() -> Self {
        Self::with_limit(MAX_RESPONSE_BYTES)
    }
}

impl EvaluationCollector {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            text: String::new(),
            complete: false,
            limit,
        }
    }
}

impl UncommittedEventCollector for EvaluationCollector {
    fn clear(&mut self) {
        self.text.clear();
        self.complete = false;
    }

    fn accept(&mut self, events: Vec<ProviderEvent>) -> Result<(), CollectionLimitExceeded> {
        for fact in events.into_iter().flat_map(|event| event.into_parts().0) {
            match fact {
                GatewayEvent::TextDelta(delta) => {
                    if delta.text.len() > self.limit.saturating_sub(self.text.len()) {
                        return Err(CollectionLimitExceeded);
                    }
                    self.text.push_str(&delta.text);
                }
                GatewayEvent::Completed(meta) => {
                    // 缺少终止原因只证明请求结束，不能据此给截断文本判分
                    self.complete = matches!(meta.finish_reason(), Some(FinishReason::Stop));
                }
                _ => {}
            }
        }
        Ok(())
    }
}
