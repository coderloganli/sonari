# auth

## 模块定位

`auth` 拥有管理员和用户认证、token 颁发、SDK API-key 验证和权限目录管理。

## 核心职责

- 管理员/用户认证
- JWT access/refresh token 颁发和验证
- 短信验证码颁发和验证
- SDK API-key 验证
- 权限目录管理
- 本地超级管理员启动引导

## 边界

### 负责

- 认证逻辑和 token 管理
- 短信验证码生成和验证
- SDK API-key 验证
- 权限目录（permission catalog）所有权
- 管理员启动引导（本地环境）

### 不负责

- 用户资料管理（由 `user` 负责）
- 内容管理
- 语音/Provider 管理
- VIP 或支付逻辑

## 内部结构

```
auth/
  src/
    lib.rs                # 模块入口
    domain/               # 领域模型
      mod.rs              # Admin, User, SmsCode, SdkApiKey, Permission
    application/          # 应用服务
      mod.rs              # AuthService, AuthAdminUseCases, AuthRuntimeUseCases
    ports/                # 端口定义
      mod.rs              # Repository traits, SmsPort, JwtPort, Clock
    adapters/             # 适配器实现
      postgres.rs         # PostgreSQL repository 实现
      sms.rs              # 短信发送实现
      jwt.rs              # JWT 实现和验证
```

## 对外接口

### AuthUseCases（用户认证）

```rust
async fn register(&self, command: RegisterCommand) -> AppResult<RegisterResult>;
async fn login(&self, command: LoginCommand) -> AppResult<LoginResult>;
async fn send_sms_code(&self, command: SendSmsCodeCommand) -> AppResult<()>;
async fn verify_sms_code(&self, command: VerifySmsCodeCommand) -> AppResult<VerifySmsCodeResult>;
async fn refresh_token(&self, command: RefreshTokenCommand) -> AppResult<RefreshTokenResult>;
```

### AuthAdminUseCases（管理员认证）

```rust
async fn admin_login(&self, command: AdminLoginCommand) -> AppResult<AdminLoginResult>;
async fn create_admin(&self, command: CreateAdminCommand) -> AppResult<AdminView>;
async fn list_admins(&self) -> AppResult<Vec<AdminView>>;
```

### AuthRuntimeUseCases（运行时验证）

```rust
async fn validate_sdk_api_key(&self, api_key: &str) -> AppResult<bool>;
async fn get_permissions(&self, user_id: i64) -> AppResult<Vec<Permission>>;
```

## 数据模型

### Admin

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| username | String | 用户名 |
| password_hash | String | 密码哈希 |
| permissions | Vec<Permission> | 权限列表 |
| created_at | DateTime<Utc> | 创建时间 |

### User

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| phone | String | 手机号 |
| password_hash | Option<String> | 密码哈希（可选） |
| created_at | DateTime<Utc> | 创建时间 |

### SmsCode

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| phone | String | 手机号 |
| code | String | 验证码 |
| expires_at | DateTime<Utc> | 过期时间 |
| used | bool | 是否已使用 |

### SdkApiKey

| 字段 | 类型 | 说明 |
|------|------|------|
| id | i64 | 主键 |
| api_key | String | API key |
| is_active | bool | 是否激活 |
| created_at | DateTime<Utc> | 创建时间 |

## HTTP API

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/auth/register` | 用户注册 |
| POST | `/auth/login` | 用户登录 |
| POST | `/auth/sms/send` | 发送短信验证码 |
| POST | `/auth/sms/verify` | 验证短信验证码 |
| POST | `/auth/refresh` | 刷新 token |
| POST | `/api/admin/auth/login` | 管理员登录 |
| POST | `/api/admin/auth/create` | 创建管理员（需认证） |
| GET | `/api/admin/auth/list` | 列出管理员（需认证） |

## 本地启动引导

对于隔离的 Rust 本地 Docker 栈，`auth` 也拥有本地超级管理员的启动引导。

- 这是一个显式的本地覆盖操作，由导入/设置流程使用
- 由 auth 拥有的启动引导密钥保护
- 正常服务启动不得变更管理员状态
- 启动引导管理员协调不依赖运行时 SMS 发送连接

## 本地 SMS 行为

`SMS_FIXED_CODE` 是本地确定性 SMS 验证的认证密钥。

- 当设置时，`auth` 持久化固定代码并跳过真实 SMS 发送
- 运行时 SMS 发送本身不支持模拟 provider

## 边界规则

- `auth` 拥有权限目录源并暴露 owner-side 管理员读取端口
- `console` 可以聚合 auth 拥有的管理员读取，但所有权保留在 `auth`
- `auth` 不扩展为通用用户中心模块

## 依赖

- `shared-kernel`: 基础类型和错误
- `platform/postgres`: PostgreSQL 持久化

## 约束

- 密码必须哈希存储
- 短信验证码有时效性（默认 5 分钟）
- JWT token 有过期时间
- API key 验证必须在运行时高效执行
