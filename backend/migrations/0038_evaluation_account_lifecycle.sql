-- 子任务停止独立于整轮停止，账号重新启用不恢复已经停止的任务
alter table evaluation_items add column stop_requested boolean not null default false;
alter table evaluation_judgments add column stop_requested boolean not null default false;
