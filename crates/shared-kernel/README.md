# shared-kernel

## 模块定位

`shared-kernel` 是跨模块共享的最小基础内核。

## 核心职责

- 通用错误码与结果类型
- Claims / tenant context / request context 基础类型
- Pagination / time / id 等基础抽象
- 少量真正稳定的共享值对象

## 边界

### 负责

- 低层公共概念
- 跨模块复用的基础类型
- 稳定的值对象

### 不负责

- 任一业务模块的核心实体
- 杂项工具堆积
- 代替 domain crate

## 内部结构

```
shared-kernel/
  src/
    lib.rs                # 模块入口
    error.rs              # 错误类型定义
    result.rs             # Result 类型别名
    context.rs            # 上下文类型（claims, request context）
    paging.rs             # 分页类型
    ids.rs                # ID 类型
    time.rs               # 时间类型
    datetime.rs           # 日期时间辅助
```

## 对外接口

### 错误类型

```rust
pub type AppResult<T> = Result<T, AppError>;

pub enum AppError {
    Internal(String),
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    // ... 其他错误变体
}

pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}
```

### 上下文类型

```rust
// JWT Claims
pub struct Claims {
    pub user_id: i64,
    pub phone: String,
    pub exp: i64,
}

// HTTP 请求上下文
pub struct HttpRequestContext {
    pub request_id: String,
    pub trace_id: Option<String>,
    pub user_id: Option<i64>,
    pub received_at: DateTime<Utc>,
}
```

### 分页类型

```rust
pub struct Pagination {
    pub page: u32,
    pub page_size: u32,
}

pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}
```

### ID 类型

```rust
pub type SessionId = i64;
pub type CharacterId = i64;
pub type SceneId = i64;
pub type VoiceprintId = i64;
pub type AgentSessionId = String;
```

### 时间类型

```rust
pub use chrono::{DateTime, Utc};

pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
}
```

## 依赖

应尽量零业务依赖。

允许的依赖：
- `chrono`: 日期时间处理
- `uuid`: UUID 生成
- `serde`: 序列化/反序列化

## 约束

- 只有多个模块都稳定需要的东西才能进入这里
- 不允许把"暂时不知道放哪"的内容放进来
- 不包含业务逻辑
- 不包含模块特定的类型

## 设计原则

1. **最小化**: 只包含真正共享的基础类型
2. **稳定性**: API 变更需要跨模块 review
3. **无业务**: 不包含任何业务逻辑
4. **避免工具堆积**: 工具函数应放在各自的模块中
