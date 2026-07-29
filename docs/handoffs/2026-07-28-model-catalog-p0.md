# 交接：模型目录数据收敛 P0（2026-07-28）

- 分支：`dev/model-catalog-p0-20260728`（commits a850c113..ba8ee277 + 本文档提交）
- 计划：`docs/superpowers/plans/2026-07-28-p0-model-catalog-data-convergence.md`
- 设计：`docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md`（§6 P0 及"P0 实施偏差记录"）

## 交付了什么

**两张新表（迁移 014 `014_provider_models_and_connections.sql`）**

- `provider_models`：权威 per-model 实体行——enabled / sort_order / tasks / traits / protocol / connection_role / params / context_limit / description / source / health。从 providers 的 6 个旧 JSON map 列 + model_profiles 回填；孤儿 profile 随回填清理。
- `provider_connections`：per-role 连接档案——base_url / auth_scheme / credentials_encrypted（加密、只写不回读）/ is_full_url / extra。providers 行自身继续充当 default 连接，此表只存附加角色。

**迁移 015 `015_drop_model_profiles.sql`**：删除 `model_profiles` 表；`ModelProfileService` 改为在 provider_models 上重实现（wire 不变，DELETE 语义见下）。

**仓储与读写路径**

- 新增 `IProviderModelRepository` / `IProviderConnectionRepository`。
- provider create/update/delete 事务内双写行表（legacy→new 方向）；删除级联两张新表。
- `ProviderResponse` 改由行投影（旧 map 列不再被读取，但仍被双写以防仓内直接读写方漂移）。
- 健康探针结果由服务端直接持久化到 `provider_models.health`（行为权威）。
- 播种闭环：provider create/update 时立即推断播种 tasks（原先仅 boot 时）。

**修复的三个数据 bug**：孤儿 profile（回填清理 + 删除级联）与 health 仅客户端写（服务端持久化）已彻底修复；克隆丢标签**部分交付**——服务端克隆端点 `POST /api/providers/{id}/clone` 已上线，**被调用时**完整保留模型行与连接档案，但设置页 UI 仍走遗留客户端克隆（`ui/src/renderer/utils/model/providerClone.ts`，把模型重建为无 profile 行），**用户可见症状在 P2 前端切换到该端点之前仍然存在**。

## Wire / API 变化

| 变化 | 说明 |
|---|---|
| `ProviderResponse.models_detail` | 新增字段 `Vec<ProviderModelResponse>`，行级模型明细；旧 map 字段照常输出（投影自行） |
| `GET/POST /api/providers/{id}/connections` | 列出 / upsert 连接档案；credentials 只写、加密存储、永不回显 |
| `DELETE /api/providers/{id}/connections/{role}` | 删除指定角色档案 |
| `GET/POST /api/provider-models`、`POST /api/provider-models/update`、`POST /api/provider-models/delete` | 行级模型目录 CRUD；update 用 double-Option 语义做部分更新 |
| `POST /api/providers/{id}/clone` | 服务端克隆，保留模型行与连接档案；被调用时修复克隆丢数据，但 UI 尚未切换到此端点（仍走遗留客户端克隆，见 P1/P2 入口第 6 项） |
| `DELETE /api/model-profiles` | **语义放宽**：现在删除的是 provider_models 目录行（原先只删 profile 覆盖层）；无存量调用方，wire 形状不变 |
| `PUT` model_health map | 旧客户端写路径仍接受（wire 兼容），但行上的 health 为服务端权威；P2 切 UI 后关闭 |

投影行为变化（T4）：旧 map 列中不属于目录（无对应 provider_models 行）的模型条目不再回显进 `ProviderResponse`（legacy 会原样回显）；`model_enabled` 只输出显式 false 条目（缺省 = 启用，与旧读方 `!= false` 语义一致）。

## 旧列冻结状态

providers 的 6 个 JSON map 列**物理保留、继续被双写、不再被读**（读路径已切行投影）。P2 收缩期物理删除。

## P1/P2 入口（见设计文档 §6 P1）

1. 新建 `crates/backend/nomifun-model-invoke` crate，搬迁 creation 的 provider.rs + adapters + 共享 HTTP 助手；
2. `AdapterRegistry` + `InvokeError` + 鉴权方案基座，删除 `is_gemini` 等名字路由启发式；
3. 探针改走 `probe()` 同管线（与真实调用统一）；
4. `ark.images`/`ark.video_jobs`、`volc.tts_v3` 或 `volc.asr_file` 落地（多连接档案端到端验证件）；
5. TTS 适配器 + `/api/tts`。
6. （P2 前端项）设置页克隆切换为调用 `POST /api/providers/{id}/clone`，替换 `ui/src/renderer/utils/model/providerClone.ts` 的遗留客户端克隆——在此之前克隆丢标签的用户可见症状仍存在。

## 验证记录（2026-07-28）

- 四个受改 crate 套件全绿：`cargo test -p nomifun-db / nomifun-api-types / nomifun-system / nomifun-ai-agent`（ai-agent 有 1 个 openclaw 既有失败，见下）；`cargo fmt --check` 干净；仓库无 clippy 严格约定。
- 全工作区 `cargo test --workspace --exclude nomifun-desktop --no-fail-fast`：36 个失败（16 套件，集中在 nomifun-app/nomifun-conversation e2e）。经与 merge-base `1d627d4b` 在原生磁盘干净 worktree 上的基线对照：**36 个全部为既有失败**（同文件行号、同 panic 文本逐字重现），**本分支引入回归为 0**；另有 2 个基线失败被本分支顺带修复（`patch_settings_unknown_field_*`、`upsert_list_resolve_delete_model_profile`）。
- 既有失败与本改动无关（ACP agent_id 校验、runtime teardown 证明、后台删除超时、终端 spawn 断言等），建议另行立项清理测试基线。

## 已记录的遗留小项（deferred minors，摘自 SDD ledger）

- T1：health 断言可钉死精确 JSON；旧 health 值损坏时缺 json() 防御；幂等 guard 未测。
- T2：set_health 会 bump updated_at（探针写扰动用户编辑时间戳语义）；set_health(None) 对已有行的清除路径未测。
- T3：create 事务 SELECT-before-write 的 SQLITE_BUSY_SNAPSHOT 模式；缺 membership+map 组合测试；缺 minified-health 存储测试；畸形 JSON 校验复用了 Conflict 错误码。
- T4：每行重复解析 health JSON；非成员模型的 map 条目不再回显（行为变化）；explicit-true 消失对仓外消费者不可验证；整表投影失败路径（capabilities 解析/解密）为既有问题。
- T5：播种回填竞态窗口（条件更新原语可加固）；GET model-profiles 可能瞬时列出未 profile 的行；health 写会移动 updated_at；DELETE 语义放宽（已在上表记录）。
- T6：DELETE 跳过 role 格式校验（无害 no-op）；POST upsert 创建时返回 200 而非 201；managed-platform 供应商也接受 connections（候选后续 guard）。
- T7：模型 create/update 未校验 connection_role（应共用 validate_role）；create connection_role 走两次写；delete-404 与 model-profiles silent-200 不一致；traits-only source 翻转未测；create TOCTOU 409-vs-404。
- T8：clone 非原子（中途失败留部分克隆，用户可删）；重复克隆产生重名（与前端行为一致）；克隆后缀恒为英文 "copy"（后续：请求可带可选名字）。
- T9：pub(crate) vs pub 可见性；boot 回填扩大到所有 unprofiled inferred 行（收敛性、已披露）；既有回填 TOCTOU；非空 inferred 跳过路径缺 spy-repo 测试。
