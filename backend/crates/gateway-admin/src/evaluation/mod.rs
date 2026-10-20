//! 原生测评的管理服务与持久任务执行；请求仍交给正常 Core 执行链

mod collector;
mod judge;
mod runner;
mod service;
mod task;

pub use service::EvaluationService;
pub use task::EvaluationTask;
