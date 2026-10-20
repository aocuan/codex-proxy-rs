-- 按账号批量读取近期测评，避免账号列表轮询扫描其他账号的历史
create index evaluation_items_account_run_idx on evaluation_items (account_id, run_id);
