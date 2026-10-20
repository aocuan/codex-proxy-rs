-- 实验记录保存引用与参数快照，不持有 Client Key 明文或账号凭据
create table evaluation_plans (
    id text primary key,
    revision bigint not null default 1 check (revision > 0),
    settings_json jsonb not null,
    next_run_at timestamptz,
    created_at timestamptz not null,
    updated_at timestamptz not null,
    check ((settings_json->>'enabled')::boolean or next_run_at is null)
);

create table evaluation_runs (
    id text primary key,
    parameters_json jsonb not null,
    profile_json jsonb not null,
    actor_json jsonb not null,
    submission_id uuid,
    -- 删除计划不删除历史实验，plan_id 保留原始来源
    plan_id text,
    scheduled_at timestamptz,
    stop_requested boolean not null default false,
    created_at timestamptz not null,
    unique (actor_json, submission_id),
    unique (plan_id, scheduled_at),
    check (
        (submission_id is not null and plan_id is null and scheduled_at is null)
        or (submission_id is null and plan_id is not null and scheduled_at is not null)
    )
);
create index evaluation_runs_created_at_idx on evaluation_runs (created_at desc, id desc);

create table evaluation_items (
    id text primary key,
    run_id text not null references evaluation_runs (id) on delete cascade,
    account_id text not null,
    repetition integer not null check (repetition between 1 and 5),
    status text not null check (
        status in ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted', 'skipped')
    ),
    request_id text,
    verdict text not null default 'ungraded' check (
        verdict in ('correct', 'wrong', 'ungraded', 'not_applicable')
    ),
    final_answer bigint check (final_answer >= 0),
    error_code text,
    message text,
    ttft_ms bigint check (ttft_ms >= 0),
    duration_ms bigint check (duration_ms >= 0),
    observation_json jsonb not null default '{}',
    started_at timestamptz,
    completed_at timestamptz,
    unique (run_id, account_id, repetition),
    check ((status in ('queued', 'running')) = (completed_at is null)),
    check (status <> 'running' or started_at is not null)
);
create index evaluation_items_queue_idx on evaluation_items (run_id, id) where status = 'queued';
create unique index evaluation_items_running_account_idx
    on evaluation_items (account_id) where status = 'running';

create table evaluation_artifacts (
    item_id text primary key references evaluation_items (id) on delete cascade,
    response_text text not null check (octet_length(response_text) <= 1048576),
    artifact_html text check (octet_length(artifact_html) <= 262144)
);
