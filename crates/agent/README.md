# agent

## 模块定位

`agent` 是 AI 对话编排模块，负责 LLM 会话管理、prompt 构建、模型调用编排和用量统计。

## 核心职责

- 管理 LLM provider 配置（conversation/assistant 两个槽位）
- 管理和渲染 prompt 模板
- 创建和管理 agent 对话会话
- 执行单轮对话（`ASR -> agent -> TTS` 中的 agent 部分）
- 生成欢迎消息
- 记录对话消息和 token 用量
- 归档已结束会话
- 发布 prompt 日志到外部事件总线

## 边界

### 负责

- Prompt 模板管理和渲染
- LLM provider 配置管理（endpoint、API key、model、temperature 等）
- Agent 会话生命周期（创建、查询、结束）
- 对话消息持久化
- LLM 调用编排（通过 `LlmGateway` 端口）
- Token 用量记录和统计
- 密钥加解密（通过 `LlmSecretsCipher` 端口）
- Prompt 日志发布

### 不负责

- Call session 生命周期（由 `call/control` 负责）
- Character/Scene 内容 CRUD（通过 `CharacterPromptContextReadPort` 只读访问）
- LLM HTTP 调用物理实现（由 `adapters/llm` 实现）
- 数据库持久化物理实现（由 `adapters/postgres` 实现）
- 密钥算法物理实现（由 `adapters/secrets` 实现）

## 内部结构

```
agent/
  src/lib.rs               # 模块入口，导出公共 API
  domain/                  # 领域模型
    mod.rs                 # ProviderKey, PromptTemplateKey, MessageRole, LlmProviderConfig,
                           # PromptTemplate, AgentSession, AgentMessage, AgentArchiveMessage,
                           # AgentMessageArchive, LlmUsageLog, SessionUsageSummary, LlmUsageStats
  application/             # 应用服务
    mod.rs                 # AgentService, AgentAdminUseCases, AgentRuntimeUseCases,
                           # UpdateAdminConfigCommand, ChatCommand, AdminConfigView
  ports/                   # 端口定义
    mod.rs                 # Repository traits, LlmGateway, LlmSecretsCipher, Clock, IdGenerator,
                           # AgentCallControlPort
  adapters/                # 适配器实现
    llm/mod.rs             # ReqwestLlmGateway (OpenAI-compatible HTTP 客户端)
    postgres.rs            # PostgreSQL repository 实现
    secrets.rs             # AES-GCM-SIV 密钥加解密实现
  context/                 # 外部上下文适配器（预留）
```

## 对外接口

### 1. Use Case 接口

#### AgentAdminUseCases（管理端）

```rust
async fn get_admin_config(&self, provider_key: &str) -> AppResult<AdminConfigView>;
async fn update_admin_config(&self, command: UpdateAdminConfigCommand) -> AppResult<AdminConfigView>;
async fn get_usage_stats(&self) -> AppResult<LlmUsageStats>;
```

#### AgentRuntimeUseCases（运行时）

```rust
async fn chat_once(&self, command: ChatCommand) -> AppResult<String>;
```

#### AgentCallControlPort（Call 控制层）

```rust
async fn create_call_session(&self, request: CreateAgentSessionRequest) -> AppResult<CreateAgentSessionResult>;
async fn generate_welcome_message(&self, agent_session_id: &str) -> AppResult<String>;
```

### 2. 端口（Port）定义

| 端口 | 职责 |
|------|------|
| `LlmProviderConfigRepository` | Provider 配置持久化 |
| `PromptTemplateRepository` | Prompt 模板持久化 |
| `AgentSessionRepository` | 会话持久化 |
| `AgentMessageRepository` | 消息持久化 |
| `AgentArchiveRepository` | 归档持久化 |
| `UsageLogRepository` | 用量日志持久化和统计 |
| `AgentSettingsRepository` | 运行时设置（recent_turns） |
| `LlmGateway` | LLM HTTP 调用抽象 |
| `LlmSecretsCipher` | 密钥加解密抽象 |
| `Clock` | 时间抽象（可测试性） |
| `IdGenerator` | 会话 ID 生成抽象 |
| `CharacterPromptContextReadPort` | Character/Scene 上下文只读（外部依赖） |

## 业务功能

### 1. Provider 配置管理

- **两个槽位**: `Conversation`（对话）和 `Assistant`（助手）
- **配置项**: endpoint_url、api_key（加密存储）、model_name、temperature、frequency_penalty
- **首次保存**: api_key 必填
- **后续更新**: api_key 可选，保留现有值

### 2. Prompt 模板管理

| ProviderKey | 模板键 |
|-------------|--------|
| Conversation | `conversation_system_1`, `conversation_system_2`, `conversation_system_3`, `conversation_welcome_user` |
| Assistant | `assistant_system` |

**模板占位符**:
- 角色: `{{name}}`, `{{persona}}`, `{{private_interests}}`, `{{personality_traits}}`, `{{speaking_style}}`, `{{occupation}}`, `{{marital_status}}`, `{{language}}`, `{{role_orientation}}`, `{{age}}`
- 模板写入路径必须拒绝旧角色占位符，例如 `{{name_zh}}`、`{{description_zh}}`、`{{traits_zh}}`、`{{sexual_preference}}`。
- 场景: `{{location}}`, `{{user_role}}`, `{{relationship}}`, `{{environment}}`, `{{goal}}`, `{{opening_event}}`
- 时间: `{{time}}`（按时区推断：凌晨/早上/下午/夜里）

### 3. 会话管理

- **创建**: 需要 caller、character_id、timezone；scene_id 可选。caller 使用全局 canonical caller identity，平台调用方为 `PlatformUser(user_id)`，SDK 调用方为 `SdkUser(partner_id, sdk_app_id, sdk_user_id, sdk_session_id, runtime_snapshot_id, external_user_id_hash)`。
- **无场景会话**: scene 为空时，prompt 渲染将场景占位符收敛为空字符串，`{{time}}` 收敛为 `未知`，不得伪造默认 scene。
- **结束**: 通过 archive 触发
- **消息**: 按 turn_number 顺序追加，保留 System/User/Assistant 角色

### 4. 对话流程

```
1. 获取会话和 provider 配置
2. 构建 system messages（模板 + character 上下文）
3. 追加最近 N 轮历史（默认 6 轮，可配置 1-20）
4. 追加当前用户消息
5. 调用 LLM
6. 记录 usage 日志
7. 发布 prompt 日志
8. 追加 assistant 消息
9. 返回回复文本
```

### 5. 欢迎消息流程

```
1. 获取会话和 character 上下文
2. 渲染 welcome_user_prompt 模板
3. 落库用户引导消息
4. 构建 system messages + 用户引导消息
5. 调用 LLM（max_tokens=120）
6. 记录 usage 和 prompt 日志
7. 落库 assistant 消息
8. 返回欢迎语
```

## 数据模型

### LlmProviderConfig

| 字段 | 类型 | 说明 |
|------|------|------|
| provider_key | ProviderKey | Conversation 或 Assistant |
| endpoint_url | String | LLM API endpoint |
| api_key_encrypted | String | 加密后的 API key |
| api_key_prefix | String | 脱敏前缀（如 `key****`） |
| model_name | String | 模型名称 |
| temperature | f64 | 采样温度 |
| frequency_penalty | f64 | 频率惩罚 |
| updated_at | DateTime<Utc> | 更新时间 |

### PromptTemplate

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| template_key | PromptTemplateKey | 模板键 |
| template_text | String | 模板文本（含占位符） |
| updated_at | DateTime<Utc> | 更新时间 |

### AgentSession

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 会话 ID（UUID） |
| caller | CallerIdentity | 平台或 SDK 调用方身份 |
| character_id | i64 | 角色 ID |
| timezone | String | 时区（如 Asia/Shanghai） |
| scene_id | Option<i64> | 可选场景 ID |
| started_at | DateTime<Utc> | 开始时间 |
| ended_at | Option<DateTime<Utc>> | 结束时间 |

### AgentMessage

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| session_id | String | 会话 ID |
| role | MessageRole | System/User/Assistant |
| content | String | 消息内容 |
| turn_number | i32 | 轮次编号 |
| created_at | DateTime<Utc> | 创建时间 |

### LlmUsageLog

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| session_id | String | 会话 ID |
| provider_key | ProviderKey | Provider 槽位 |
| model_name | String | 模型名称 |
| prompt_tokens | i32 | Prompt token 数 |
| completion_tokens | i32 | Completion token 数 |
| is_error | bool | 是否错误 |
| error_message | String | 错误信息 |
| created_at | DateTime<Utc> | 创建时间 |

### LlmUsageStats

| 字段 | 类型 | 说明 |
|------|------|------|
| total_calls | i64 | 总调用次数 |
| total_prompt_tokens | i64 | 总 prompt tokens |
| total_completion_tokens | i64 | 总 completion tokens |
| total_errors | i64 | 总错误数 |
| error_rate | f64 | 错误率 |

## HTTP API

由 `crates/api/src/agent.rs` 暴露（管理员认证）：

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/admin/agent/config/conversation` | 获取对话 provider 配置 |
| PUT | `/api/admin/agent/config/conversation` | 更新对话 provider 配置 |
| GET | `/api/admin/agent/config/assistant` | 获取助手 provider 配置 |
| PUT | `/api/admin/agent/config/assistant` | 更新助手 provider 配置 |
| GET | `/api/admin/agent/usage` | 获取用量统计 |

## 适配器实现

| 适配器 | 位置 | 实现端口 |
|--------|------|----------|
| `ReqwestLlmGateway` | `agent/adapters/llm` | `LlmGateway`（OpenAI-compatible HTTP） |
| `Postgres*Repository` | `agent/adapters/postgres` | 所有 Repository traits |
| `AesGcmSivCipher` | `agent/adapters/secrets` | `LlmSecretsCipher` |

## 技术约束

- **模板文本**: 统一转换为 LF 换行（`\\r\\n` → `\\n`）
- **Recent turns**: 默认 6 轮，可配置范围 1-20
- **Temperature**: 默认 0.7，输入 0 时自动使用默认值
- **时间推断**: 只有 `time_period_mode=auto` 时按时区推断，`fixed` 使用 scene 配置，`disabled` 返回"未知"

## 依赖

- `shared-kernel`: 基础类型和错误
- `character-context`: Character/Scene 上下文只读端口
- `reqwest`: HTTP 客户端
- `aes-gcm-siv`: 密钥加解密
- `chrono-tz`: 时区处理

## 约束

- Prompt 逻辑必须集中在本模块，其他模块不得自行拼 prompt
- Character 依赖通过只读端口，不得镜像 SQL schema
- API key 加密存储，只返回脱敏前缀
- 会话 ID 由 `IdGenerator` 生成，应用层不感知具体格式
