# app

## 模块定位

`app` 是整个 Rust 后端的唯一 composition root（组合根）。

它不是业务模块，而是装配模块。

## 核心职责

- 加载和校验配置
- 初始化 tracing / logging / metrics
- 初始化 PostgreSQL、Redis、Storage 等共享资源
- 初始化各模块的 adapters 和 use cases
- 装配 HTTP router
- 启动后台 worker / jobs
- graceful shutdown

## 边界

### 负责

- 进程级初始化
- 依赖注入
- 运行时资源生命周期管理
- 模块装配和编排

### 不负责

- 业务规则
- SQL 执行
- DTO 定义
- provider 协议实现

## 内部结构

```
app/
  src/
    lib.rs                # 模块入口
    bootstrap.rs          # 服务启动引导
    config.rs             # 配置加载和校验
    runtime.rs            # 运行时资源管理
    router.rs             # 路由装配
    workers.rs            # 后台 worker 启动
    shutdown.rs           # 优雅关闭
```

## 启动流程

```
1. 加载配置文件（环境变量 + 配置文件）
2. 初始化 tracing / logging / metrics
3. 初始化共享资源：
   - PostgreSQL 连接池
   - Redis 客户端
   - Storage 客户端
4. 初始化各模块 adapters
5. 装配各模块 use cases
6. 装配 HTTP router（api 层）
7. 启动后台 workers
8. 启动 HTTP server
9. 等待关闭信号
10. 优雅关闭所有资源
```

## 依赖

### 直接依赖

- `api`: HTTP 路由层
- `observability`: 可观测性初始化
- `shared-kernel`: 基础类型
- `platform/postgres`: PostgreSQL 支持
- `platform/redis`: Redis 支持
- `platform/storage`: Storage 支持
- `auth`: 认证模块
- `user`: 用户模块
- `character`: 角色模块
- `voice`: 语音模块
- `agent`: Agent 模块
- `call`: 通话模块

## 对外接口

- `main.rs`: 进程入口
- `bootstrap()`: 服务启动函数
- `test_bootstrap()`: 测试环境启动函数

## 约束

- 所有依赖装配必须集中在这里完成
- 不能把业务规则堆回 `app`
- 不得出现模块绕过 `app` 自行全局初始化资源
- 所有共享资源必须在 `app` 层初始化后通过依赖注入传递给各模块
