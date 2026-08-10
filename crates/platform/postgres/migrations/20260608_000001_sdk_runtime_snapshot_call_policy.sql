-- 为 sdk_runtime_snapshots 增加 call_policy 列,使呼叫策略随快照一并落库。
-- 此前 call_policy 仅在创建快照时由配置派生、未入库,二次读取(select *)时
-- 被硬编码为缺省 false,导致"写真值、读 false"的漂移。新增列后读写同源,快照
-- 忠实记录下发时的决策。
--
-- 列语义:可空 jsonb,缺省语义为 false。NULL(历史行)由读路径
-- map_runtime_snapshot 归一为 {"require_bluetooth_before_call": false},
-- 与 runtime_call_policy 对缺失/null 的处理保持一致。
alter table sdk_runtime_snapshots
  add column if not exists call_policy jsonb;
