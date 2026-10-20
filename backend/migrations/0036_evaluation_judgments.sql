-- 判题使用独立请求，生成结果保留后可以单独重试评审
create table evaluation_judgments (
    id text primary key,
    item_id text not null references evaluation_items (id) on delete cascade,
    account_id text not null,
    attempt integer not null check (attempt between 1 and 5),
    status text not null check (status in ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted')),
    request_id text,
    verdict text not null default 'ungraded' check (verdict in ('correct', 'wrong', 'ungraded')),
    reason text check (octet_length(reason) <= 4096),
    response_text text check (octet_length(response_text) <= 65536),
    observation_json jsonb not null default '{}',
    created_at timestamptz not null,
    completed_at timestamptz,
    unique (item_id, attempt),
    check ((status in ('queued', 'running')) = (completed_at is null)),
    check (status = 'succeeded' or verdict = 'ungraded')
);
create unique index evaluation_judgments_running_account_idx
    on evaluation_judgments (account_id) where status = 'running';
create index evaluation_judgments_queue_idx on evaluation_judgments (created_at, id) where status = 'queued';
