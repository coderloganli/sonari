# api

## 模块定位

`api` 是 HTTP 和内部协议适配层，负责请求路由、验证、认证中间件装配和 DTO 映射。

## 核心职责

- 路由注册和 HTTP 端点暴露
- 请求解析和验证
- 认证中间件装配
- 可观测性 HTTP 中间件装配
- DTO 映射和错误/响应映射
- 内部协议端点（面向 Worker）

## 边界

### 负责

- HTTP 路由组织
- 请求/响应 DTO 定义
- 认证中间件应用
- 请求上下文注入（由 `observability` 提供）
- 调用各模块暴露的 use case 或 port 契约

### 不负责

- 数据库访问
- Provider 调用
- 跨模块业务编排
- 业务规则实现

## 内部结构

```
api/
  src/
    lib.rs                # 模块入口
    middleware.rs         # 认证中间件
    dto/                  # 请求/响应 DTO
    routes/               # 路由处理器
      mod.rs
      auth.rs             # 认证相关路由
      user.rs             # 用户相关路由
      character.rs        # 角色相关路由
      voice.rs            # 语音相关路由
      agent.rs            # Agent 配置路由
      call.rs             # 通话相关路由
      console.rs          # 管理员控制台路由
      internal.rs         # 内部协议路由（Worker）
```

## 路由分组

### 公开端点（面向客户端）

| 模块 | 路由前缀 | 说明 |
|------|----------|------|
| auth | `/auth/*` | 登录、注册、短信验证码 |
| user | `/user/*` | 用户资料、通知 |
| character | `/character/public/*` | 公开角色信息 |
| call | `/call/*` | 通话启动/结束/历史 |

### 管理员端点

| 模块 | 路由前缀 | 说明 |
|------|----------|------|
| auth | `/api/admin/auth/*` | 管理员认证 |
| user | `/api/admin/users/*` | 用户管理 |
| character | `/api/admin/character/*` | 角色内容管理 |
| voice | `/api/admin/voice/*` | 语音供应商管理 |
| agent | `/api/admin/agent/*` | LLM 配置管理 |
| call | `/api/admin/call/*` | 通话日志查询 |
| console | `/api/admin/console/*` | 控制台聚合视图 |

### 内部端点（面向 Worker）

| 路由前缀 | 说明 |
|----------|------|
| `/internal/runtime/*` | Worker 轮询/状态报告 |
| `/internal/speech/*` | 语音会话管理 |

## 认证中间件

- 管理员端点必须通过认证中间件保护
- 本地内容导入路径通过 `/api/admin/login` 登录后访问受保护端点
- 本地管理员 API 不是未认证的例外

## 请求上下文

- 全站 request-id / trace 相关性由 `observability` 拥有
- `api` 只负责挂载中间件
- Handler 从 request extensions 读取 `HttpRequestContext`（当 use case 需要时）

## 边界规则

- Handler 只调用 owner 提供的 use-case 或 port 契约
- 不在已有共享模块契约时发明备用协议契约
- 只读控制台路由保留在 `console`，auth/user mutations 保留在各自 owner 路由

## 依赖

- `shared-kernel`: 基础类型和错误
- `observability`: HTTP 中间件和请求上下文
- `auth`: 认证相关 use cases
- `user`: 用户相关 use cases
- `character`: 角色相关 use cases
- `voice`: 语音相关 use cases
- `agent`: Agent 配置 use cases
- `call`: 通话相关 use cases
- `console`: 控制台聚合 use cases

## 约束

- 不得绕过 `api` 层直接暴露业务模块内部类型
- DTO 必须与领域模型解耦
- 认证检查必须在路由层完成，不得下沉到业务层
