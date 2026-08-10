# voice

## 模块定位

`voice` 拥有语音供应商配置、路由、输入语言配置和 provider 能力分发。

## 核心职责

- ASR/TTS 供应商 CRUD
- 激活供应商选择
- ASR 语言路由
- ASR 输入语言配置
- Runtime provider 范围固定为：
- `Alibaba` 用于 ASR
- `ElevenLabs` 用于 TTS 和 voice catalog
- 通过 provider-adapter 边界使用激活供应商的真实 provider 凭证列出 TTS voice catalog
- 会话级 runtime ASR session 由 `voice` 持有和管理
- 窄的 runtime 语音执行端口用于后端语音编排

## 边界

### 负责

- 供应商配置管理
- ASR 语言路由配置
- ASR 输入语言配置
- TTS voice catalog 管理
- Provider 能力分发
- 窄的 runtime 语音执行端口

### 不负责

- Call lifecycle 编排
- Worker/runtime 编排
- 角色内容管理
- Transport 选择
- 词汇或热词管理

## 内部结构

```
voice/
  src/
    lib.rs                # 模块入口
    domain/               # 领域模型
      mod.rs              # VoiceSupplier, AsrRoute, AsrLanguage, TtsVoice
    application/          # 应用服务
      mod.rs              # VoiceService, VoiceAdminUseCases
    ports/                # 端口定义
      mod.rs              # Repository traits, provider ports, runtime port
    adapters/             # 适配器实现
      mod.rs
      alibaba.rs          # 阿里云 ASR/TTS 适配
      elevenlabs.rs       # ElevenLabs TTS 适配
      postgres.rs         # PostgreSQL repository 实现
```

## 对外接口

### VoiceAdminUseCases（管理端）

```rust
// 供应商管理
async fn create_supplier(&self, command: CreateSupplierCommand) -> AppResult<SupplierView>;
async fn update_supplier(&self, command: UpdateSupplierCommand) -> AppResult<SupplierView>;
async fn list_suppliers(&self) -> AppResult<Vec<SupplierView>>;
async fn set_active_supplier(&self, command: SetActiveSupplierCommand) -> AppResult<()>;

// ASR 路由
async fn create_asr_route(&self, command: CreateAsrRouteCommand) -> AppResult<AsrRouteView>;
async fn update_asr_route(&self, command: UpdateAsrRouteCommand) -> AppResult<AsrRouteView>;
async fn list_asr_routes(&self) -> AppResult<Vec<AsrRouteView>>;
async fn delete_asr_route(&self, route_id: i64) -> AppResult<()>;

// ASR 输入语言
async fn update_asr_input_language(&self, command: UpdateAsrInputLanguageCommand) -> AppResult<AsrInputLanguageView>;
async fn get_asr_input_language(&self) -> AppResult<AsrInputLanguageView>;

// TTS voice catalog
async fn list_tts_voices(&self) -> AppResult<Vec<TtsVoiceView>>;
async fn sync_tts_voices(&self) -> AppResult<Vec<TtsVoiceView>>;
```

### VoiceRuntimeExecutionPort（运行时端口）

```rust
pub trait VoiceRuntimeExecutionPort: Send + Sync {
    async fn open_asr_session_for_runtime(...);
    async fn push_asr_audio_for_runtime(...);
    async fn commit_asr_session_for_runtime(...);
    async fn poll_asr_events_for_runtime(...);
    async fn close_asr_session_for_runtime(...);

    async fn stream_tts_for_runtime(...);
}
```

## 数据模型

### VoiceSupplier

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| name | String | 供应商名称 |
| provider_type | ProviderType | Alibaba/ElevenLabs |
| api_endpoint | String | API endpoint |
| api_key_encrypted | String | 加密的 API key |
| is_active | bool | 是否激活 |
| created_at | DateTime<Utc> | 创建时间 |

### AsrRoute

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| pattern | String | 路由模式（通配符） |
| supplier_id | i64 | 关联供应商 ID |
| priority | i32 | 优先级 |
| created_at | DateTime<Utc> | 创建时间 |

### AsrInputLanguage

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键（单例） |
| language | SpeechInputLanguage | Zh/En |
| updated_at | DateTime<Utc> | 更新时间 |

### TtsVoice

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| voice_id | String | Provider voice ID |
| name | String | 语音名称 |
| language | String | 语言 |
| gender | Option<String> | 性别 |
| description | Option<String> | 描述 |
| is_active | bool | 是否激活 |

## Provider 类型

```rust
pub enum ProviderType {
    Alibaba,    // 用于 ASR
    ElevenLabs, // 用于 TTS
}

pub enum SpeechInputLanguage {
    Zh,
    En,
}

pub enum AsrLanguage {
    Zh,
    En,
}
```

## HTTP API

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/admin/voice/supplier/create` | 创建供应商 |
| PUT | `/api/admin/voice/supplier/update` | 更新供应商 |
| GET | `/api/admin/voice/supplier/list` | 列出供应商 |
| PUT | `/api/admin/voice/supplier/set-active` | 设置激活供应商 |
| POST | `/api/admin/voice/asr-route/create` | 创建 ASR 路由 |
| PUT | `/api/admin/voice/asr-route/update` | 更新 ASR 路由 |
| GET | `/api/admin/voice/asr-route/list` | 列出 ASR 路由 |
| DELETE | `/api/admin/voice/asr-route/:id` | 删除 ASR 路由 |
| PUT | `/api/admin/voice/input-language` | 更新 ASR 输入语言 |
| GET | `/api/admin/voice/input-language` | 获取 ASR 输入语言 |
| GET | `/api/admin/voice/tts-voices` | 列出 TTS voices |
| POST | `/api/admin/voice/tts-voices/sync` | 同步 TTS voices |

## 边界规则

- `voice` 拥有 supplier/config/routing 并暴露窄的 runtime 执行端口
- TTS supplier 只拥有 provider 凭证、endpoint、model；角色级 `voice_id` 不在 supplier 上
- Runtime TTS 必须按 `character -> voiceprint -> voiceprint_tts_bindings(provider)` 解析 provider voice id
- `speech-runtime` 通过该窄端口消费 runtime 执行，但不直接持有 provider socket
- Runtime TTS 以 PCM chunk stream 输出；不得等待整段音频合成完成后再返回给通话链路
- 路由验证、runtime preflight 和 runtime dispatch 共享相同的语言感知 ASR 能力映射
- Admin API 拥有 supplier config、ASR routes、ASR input language 和 TTS voice listing 的读/写入口点
- Runtime ASR 必须使用“当前语言路由 provider 对应的激活 supplier”，不能直接按 provider 命中未激活 supplier
- Runtime ASR 的真实 WebSocket 连接只能由 `voice` 持有；`speech-runtime` 只持有 session handle
- Runtime ASR 的真实 WebSocket 连接是 owner-instance 绑定状态，不是 backend 实例间可迁移的共享状态
- Runtime ASR 事件必须保留显式 round 关联；不得把 transcript 轮次归属退化为 FIFO 顺序假设
- 若 speech-runtime 流量落到非 owner backend 实例，系统必须显式报 owner mismatch，而不是静默重连或伪造 session missing

## Runtime 约束

- Runtime provider 范围固定：
  - `Alibaba` 用于 ASR
  - `ElevenLabs` 用于 TTS 和 voice catalog
- 如果 ASR 语言路由指向的 provider 没有激活且配置完整的 supplier，runtime 必须返回明确配置错误
- Alibaba runtime ASR 使用 DashScope realtime WebSocket，会话级持续连接由 `voice` 管理
- Alibaba runtime ASR 当前只支持 `qwen3-asr-flash-realtime` 的 Qwen ASR Realtime Manual 模式：
  - `model_name` 为空时使用默认 `qwen3-asr-flash-realtime`
  - `model_name` 或 `endpoint_url` query 中显式配置 model 时，必须等于 `qwen3-asr-flash-realtime`
  - `session.turn_detection = null`
  - `input_audio_buffer.commit` 由 `speech-runtime` 显式触发
  - `round_id` 是 runtime ASR 正确性主轴，provider `item_id` 不参与状态迁移、错误判断或轮次归属
  - `input_audio_buffer.committed` 只表示 provider 已接受 commit；即使缺少 `item_id` 也不得让 session 失败
  - provider 若在第一轮 commit 之前就先返回 early partial / final / failure，必须先在 `voice` 内部暂存；第一轮 commit 到达后归属到该 `round_id`
  - 第一轮 commit 之后，任何没有当前 unresolved round 的 provider transcript/failure 都不得绑定到下一轮；transcript 只能降级为 warning，failure 只能作为 session 级失败处理
  - 同一 ASR session 同一时刻最多只能存在一个 unresolved round；该提交节奏由 `speech-runtime` 拥有并保证，`voice` 只做 provider-local 防御性校验
- 打开 ASR session 的 backend instance 必须在整个 speech session 生命周期内保持 owner 身份
- provider 断线时必须返回明确 runtime failure，不允许 silent drop
- Voice catalog listing 使用激活供应商的真实 provider 凭证

## 依赖

- `shared-kernel`: 基础类型和错误
- `platform/postgres`: PostgreSQL 持久化

## 约束

- API key 必须加密存储
- 同一时间只有一个激活的供应商
- ASR 路由按优先级匹配
- ASR 输入语言是全局单例配置
- TTS voice 列表从 provider 同步，不支持手动创建
