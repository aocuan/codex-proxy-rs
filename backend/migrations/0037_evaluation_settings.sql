-- 默认设置只影响新建草稿，TypeSafe 凭据独立于实验快照和公共设置响应
create table evaluation_settings (
    id integer primary key check (id = 1),
    revision bigint not null check (revision > 0),
    defaults_json jsonb not null,
    typesafe_api_key text
);

-- 外部判题不伪造网关账号；仍占用判题队列的一个并发位置
alter table evaluation_judgments alter column account_id drop not null;
drop index evaluation_judgments_running_account_idx;
create unique index evaluation_judgments_running_account_idx
    on evaluation_judgments (coalesce(account_id, 'typesafe')) where status = 'running';
