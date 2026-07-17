# Janex Program Architecture

本文定义 Janex 程序自身的目标架构。Janex 面向用户提供一站式 Java 工作站体验，包括 SDK 管理、应用管理、依赖获取、Java 脚本运行，以及未来的项目管理和构建能力。

本文不定义被构建项目的模型、构建 DSL、任务图、变体选择或缓存键。这些内容应由独立的构建系统设计文档描述。

`.janex` 文件格式由 [`spec/FileFormat.md`](spec/FileFormat.md) 单独定义；命令行行为由 [`spec/CLI.md`](spec/CLI.md) 描述。

## Architecture Summary

Janex 采用以下总体结构：

> 一个不依赖 Java 的 native workstation host，若干并列的领域服务，共享获取、验证、存储、策略、工具链和进程管理能力，并按需启动隔离的 JVM 工具及用户进程。

一站式用户体验不要求所有能力共享同一个领域模型、插件机制或进程。Janex 统一管理内容、来源、信任、工具链和进程生命周期，但保留 SDK、Application、Dependency、Script 与 Project/Build 各自的语义和状态。

```text
CLI / Shell Shim / Future GUI and IDE
                    |
                    v
              Janex Host API
                    |
        +-----------+-----------+-----------+-----------+
        |           |           |           |           |
        v           v           v           v           v
   SDK Service  App Service  Dependency  Script     Project
                              Service     Service    Service
        |           |           |           |           |
        +-----------+-----------+-----------+-----------+
                    |
                    v
 Config / Acquisition / Store / Policy / Credentials
 Toolchain Selection / Process Supervision / Events
                    |
        +-----------+--------------------+
        |                                |
        v                                v
 Trusted JVM Tools and Backends     User Code Processes
```

## Architectural Principles

### Native Bootstrap

Janex Host 必须能够在没有任何 Java runtime 的环境中启动，并完成配置读取、网络访问、下载校验、安全解包、SDK 安装、状态恢复和进程启动。Java SDK provider 的基本工作不得依赖一个已经存在的 JVM。

### Modular Monolith First

Janex 初始形态是 native 模块化单体，而不是一组本地微服务。配置、Store、安装事务和 Registry 共享紧密的事务边界，把它们拆成多个常驻服务会增加协议、升级、锁竞争和故障恢复成本。

进程拆分只用于建立有实际意义的语言、生命周期、故障或信任边界。

### One Host, Multiple Domains

SDK、Application、Dependency、Script 和 Project/Build 是并列领域。未来的构建能力不得成为其他产品能力的底层抽象；SDK 安装、应用更新和依赖下载也不得伪装成普通 build task。

### Shared Mechanisms, Separate Semantics

各领域共享下载、校验、不可变内容存储、事务、策略、凭据、工具链选择和进程监督等机械能力，不共享万能的 `Package`、`Resolver`、`Repository`、`Installation` 或 `Lock` 模型。

### Explicit State Ownership

每类持久状态都有明确所有者。只有 Janex Host 可以提交全局状态变更；JVM helper、build backend、用户程序和第三方代码不能直接修改全局 Registry、Store metadata 或用户配置。

### Optional Acceleration

Daemon、文件监听、warm worker 和内存缓存只能作为可丢弃的性能层。任何命令在没有 daemon 时仍必须保持相同的正确性语义。

## Frontends and Host API

`janex` CLI 是 Janex Host 的前端，不是业务逻辑的所有者。Shell shim、未来的 GUI、IDE 集成和 daemon RPC 都应调用相同的 application services。

```text
CLI --------+
Shell Shim -+
IDE --------+--> Janex Host API
GUI --------+
Daemon RPC -+
```

Host API 使用 typed request、typed result 和 structured event。领域服务不得直接输出 ANSI 文本、读取终端输入、终止进程或假设调用者一定是 CLI。

一个典型的可变更操作遵循以下流程：

```text
Request
  -> Resolve and Plan
  -> Policy Evaluation
  -> Approval Request, if required
  -> Transactional Apply
  -> Structured Result
```

操作上下文应支持：

- `OperationId`；
- cancellation；
- progress events；
- diagnostics；
- approval requests；
- offline/frozen policy；
- human-readable 和 machine-readable rendering 的分离。

CLI 的 `--json` 输出必须由 typed result 渲染，不能通过解析人类日志生成。未来的 IDE 或 GUI 也不应通过运行 CLI 并解析终端文本来复用业务能力。

## Domain Services

### SDK Service

SDK Service 负责：

- managed SDK 的安装、更新和卸载；
- external SDK 的只读发现和 fingerprint；
- SDK identity、alias 和默认选择；
- capability probing；
- installation generation 和 provenance。

SDK Service 管理可用工具，但不负责为每个消费者决定最终工具链。

### Toolchain Service

Toolchain Service 是共享能力，根据显式 requirement 选择一个已安装或外部发现的 SDK：

```text
ToolchainRequirement
  -> ToolchainResolver
  -> ToolchainBinding
```

Application、Script 和 Project/Build 都通过同一服务选择 Java。SDK 命令负责改变安装集合，Toolchain Service 负责读取集合并解释选择结果。

需要区分以下 runtime role，即使它们最终绑定到同一个 JDK installation：

- JVM tools 或 build engine 使用的 engine runtime；
- 编译脚本或项目使用的 target toolchain；
- 运行最终应用使用的 application runtime。

### Application Service

Application Service 负责：

- 应用获取、验证和安装；
- publisher、source 和 provenance；
- application generation；
- active generation 的原子切换；
- update 和 rollback；
- exposed command registry；
- runtime requirement 和启动计划。

应用启动只消费已安装结果或显式指定的本地 artifact，不求值项目构建配置，也不加载 build plugin。

`.janex` 是 Application Service 可消费的一种 application bundle。Application Service 通过格式适配器调用 `janex-format`，而不是把文件格式模型扩散到整个程序。

### Dependency Service

Dependency Service 负责：

- repository metadata；
- domain-specific dependency resolution；
- resolved graph 和 provenance；
- lock snapshot；
- artifact acquisition 和 materialization。

Library 通常不是全局安装的软件。其逻辑状态属于请求它的 Application、Script 或 Project；全局层只共享不可变 artifact 内容和可重新获取的 metadata cache。同一个 Library 的多个版本可以自然共存。

不同领域可以共享 repository transport、metadata cache、offline policy 和 diagnostics，但 Java SDK version selection、Maven dependency mediation、Application channel selection 和 build plugin compatibility 使用各自的解析算法。

### Script Service

Script Service 负责：

- local 或 remote source identity；
- script directives 和 normalized manifest；
- dependency lock；
- toolchain requirement；
- compilation cache identity；
- execution plan；
- remote source trust。

Script Service 组合 Dependency Service、Toolchain Service、Compiler capability 和 Process Supervisor。它可以把脚本降低为内部编译请求，但不能依赖未来构建引擎的内部项目模型。

### Project Service

Project Service 是未来项目管理和构建能力在 Janex Host 中的产品级入口。它负责项目发现、backend 选择、环境准备、操作编排和结构化结果，但不拥有全局 SDK、Application、Store 或 Credential 状态。

Project Service 可以支持并列 backend：

```text
GradleBackend
MavenBackend
JanexNativeBackend
```

这些 backend 可以统一高层操作，例如 `detect`、`sync`、`build`、`test`、`run` 和 `model`，但不要求把 Gradle、Maven 和 Janex 的内部构建图翻译成同一种模型。

## Shared Capabilities

### Acquisition

SDK、Application、Library、remote Script、internal helper 和未来 build plugin 共享同一套获取管线：

```text
Resolve Source Metadata
  -> Acquisition Plan
  -> Policy Evaluation
  -> Download
  -> Integrity and Identity Verification
  -> Safe Extraction or Materialization
  -> Immutable Store Commit
  -> Domain Registry Commit
```

Provider 只返回候选 identity、version、platform、URL、digest、signature、archive layout 和 provenance 等结构化信息。Provider 不得自行写 Store、读取任意 credential、执行 shell、运行 post-install hook 或绕过 policy。

### Content Store

Content Store 只理解不可变的 blob、tree、realization、root、lease 和 transaction，不理解 SDK default、Application generation 或 Project workspace 等领域概念。

推荐使用 semantic roots、temporary leases 和 mark-and-sweep GC，而不是让各领域直接操作文件或维护脆弱的逐对象引用计数。

统一提交流程为：

```text
Write to Staging
  -> Verify
  -> Normalize
  -> Commit Immutable Objects
  -> Commit Semantic Root
```

崩溃后可以遗留未引用的不可变对象并由 GC 回收，但 Registry 不得指向尚未提交的对象。

### Policy and Credentials

Policy Engine 评估结构化 effect，例如 network access、code execution、native code、Java agent、credential use 和 global command exposure。它不调用具体领域服务，也不把一次模糊确认扩展为无限范围的永久信任。

Credential Broker 保存和解析 `SecretRef`。Secret value 只在执行或网络请求前的最后阶段注入，不进入 lock、日志、cache key 或持久化 execution plan。

### Process Supervision

所有进程启动先生成不可变的 `ExecutionPlan` 或底层 `ProcessSpec`，再由 Process Supervisor 执行。

计划至少描述：

- executable 和 arguments；
- working directory；
- environment delta；
- stdin/stdout/stderr mode；
- toolchain binding；
- temporary 或 materialized inputs；
- timeout 和 cancellation；
- process-tree termination；
- secret references。

Application、Script、build backend、JVM helper 和 shell shim 可以共享 Process Supervisor，但各自使用不同的领域 planner 和 policy profile。

## Process Model

### Native Host Process

Native Host 是全局状态和产品规则的唯一权威。初始实现可以由 CLI 进程直接托管，不要求 daemon。

### Trusted JVM Tools

适合复用 JVM 生态库的能力可以运行在 Janex 签名和版本化的 JVM tools process 中，例如：

- Maven Resolver integration；
- Java compiler integration；
- test discovery；
- Kotlin compiler integration；
- 未来的 Janex native build engine。

Native Host 不通过 JNI 嵌入 JVM。跨进程通信使用粗粒度、版本化协议，并支持 handshake、capability negotiation、request ID、structured events、diagnostics、cancellation 和 staging output references。

JVM tools process：

- 不拥有全局状态；
- 不直接写全局数据库；
- 不直接提交 Store metadata；
- 不加载第三方项目代码到可信进程；
- 可以随 operation 结束而退出；
- 可以在未来按精确 runtime、helper digest 和 policy 建立可回收 worker pool。

### User and Plugin Processes

以下代码始终在独立进程中运行：

- Java applications；
- compiled scripts；
- tests；
- annotation processors；
- compiler plugins；
- Gradle 或 Maven build logic；
- future third-party Janex plugins。

进程隔离提供故障、classpath 和生命周期隔离，但不能自动等同于安全 sandbox。Janex 不应宣称尚未由操作系统强制实现的权限限制。

### Optional Daemon

未来的 `janexd` 可以提供下载去重、metadata 热缓存、文件监听、IDE 长连接、后台 operation 和 warm worker 等性能能力。

Daemon 不得成为事实来源或正确性依赖：

- daemon 不存在时命令仍可执行；
- daemon crash 不破坏安装状态；
- 版本不兼容时可以重启或回退；
- 所有重要状态仍保存在 transaction、Registry 和 immutable Store 中；
- 前台应用和脚本不必由 daemon 持有 TTY 和进程生命周期。

## State Model

Janex 的状态按所有权和生命周期分层：

| State | Purpose | Lifecycle |
| --- | --- | --- |
| User configuration | repositories、profiles、defaults、policy | 人类维护，不能被后台静默重写 |
| Global state database | installations、generations、aliases、roots、operations、schema version | Host 管理，事务更新 |
| Immutable content store | verified blobs、trees、realizations | digest-addressed，由 roots 和 leases 管理 |
| Metadata cache | catalog 和 repository metadata | 可重新获取，具有 freshness 语义 |
| Script cache | compiled script results | TTL lease 或显式 pin |
| Build cache | future derived build outputs | 可丢弃，与安装状态隔离 |
| Workspace state | project-local derived state | 位于 workspace，本地可重建 |
| Staging | incomplete operations and process outputs | operation-scoped，失败后回收 |
| Credentials | secret material | 由 OS credential store 或独立 broker 管理 |

底层 CAS 实现可以复用，但不同状态不能共享相同的 trust、retention 和 GC 规则：

- SDK installation 和 Application generation 是长期 root；
- Library artifact 通常是可重新获取的缓存；
- Script result 使用短期 lease；
- Build output 属于 workspace 或 build cache；
- Download staging 只在 operation 期间存活；
- External SDK 不进入 Store，只保存只读 discovery record 和 fingerprint。

全局状态可以使用一份 SQLite database，但每个领域拥有自己的 repository、table 和 migration。领域服务不得绕过 repository 直接跨域写表。

## Module Boundaries

以下名称表示目标逻辑边界，不要求初始实现立即为每个边界建立独立 crate：

```text
janex-cli
janex-host
janex-sdk
janex-app
janex-dependency
janex-script
janex-project
janex-acquisition
janex-toolchain
janex-policy
janex-process
janex-store
janex-format
janex-protocol
```

依赖方向为：

```text
Frontends
  -> Host Application Services
  -> Domain Services
  -> Capability Interfaces

Concrete Adapters
  -> Capability Interfaces
```

Concrete adapter 只在 composition root 组装。Domain Service 测试可以使用 fake capability，不依赖真实网络、用户全局目录或终端。

`janex-format` 是独立的格式 codec，只负责 `.janex` 编解码、格式校验、格式级条件、checksum 和 compression。它不依赖 SDK、Application、Dependency、Script、Project、CLI、网络或全局 Store。

需要禁止以下反向依赖：

```text
store       -> sdk/app/script/project types
acquisition -> Maven/SDK/Application resolution semantics
format      -> Application Service or build engine
sdk         -> CLI or project subsystem
script      -> build engine internals
application -> project configuration evaluator
policy      -> concrete domain services
JVM tools   -> Host database internals
plugin      -> global Store, config or credential handles
```

## Shared and Domain-Specific Types

适合共享的类型是稳定的机械原语：

```text
Digest
BlobRef
TreeRef
Platform
DownloadRequest
VerifiedBlob
SecretRef
ProcessSpec
TransactionId
Lease
OperationId
StructuredEvent
```

以下概念应保留领域类型，不能仅因为字段相似就统一：

```text
SdkIdentity
ApplicationIdentity
MavenCoordinate
ScriptIdentity
ProjectIdentity
SdkVersion
ApplicationVersion
DependencyConstraint
SdkInstallation
ApplicationGeneration
DependencyLock
ScriptLock
ProjectLock
```

新增公共抽象前，应确认多个领域确实共享相同的不变量和状态机，而不只是拥有相似名称。

## Versioning Boundaries

Janex 的以下版本必须相互独立：

- native host version；
- local RPC protocol version；
- JVM tools protocol 和 implementation version；
- future build engine version；
- persistent state schema version；
- catalog metadata schema version；
- lockfile schema version；
- `.janex` file format version。

Native Host 管理 helper 和 backend 的选择、下载、验证与生命周期。未来项目固定构建语义时，应固定 build engine 和相关协议输入，而不是要求 SDK、Application 和整个 Host 跟随项目构建模型一起版本化。

## Architectural Invariants

1. Janex Host 在没有 Java runtime 时仍能安装和管理 SDK。
2. CLI、GUI、IDE、shim 和 daemon 复用相同的 Host application services。
3. SDK、Application、Dependency、Script 和 Project/Build 是并列领域。
4. 领域共享机械基础设施，不共享万能 package model。
5. Library 的逻辑所有者是 Application、Script 或 Project，而不是全局 installation registry。
6. Toolchain selection 与 SDK installation workflow 分离。
7. 只有 Host 可以提交全局状态变更。
8. 所有 Store content 不可变并由 digest 标识。
9. Registry transaction 不得引用未提交的 Store object。
10. Application runtime 不求值项目配置，也不加载 build plugin。
11. Build backend 无权修改全局 SDK 或 Application state。
12. 用户代码和第三方代码不进入 Native Host 或可信 JVM tools process。
13. Daemon、worker 和内存缓存不是正确性依赖。
14. `.janex` format codec 不依赖产品领域和全局状态。
15. File format、Host、protocol、state schema 和 build engine 分别版本化。

## Deferred Design Areas

本架构只为以下能力保留边界，不在本文中规定其内部设计：

- Janex project manifest；
- build DSL；
- task、action、target 和 variant model；
- incremental compilation；
- local 或 remote build cache；
- public plugin API；
- Build Server Protocol integration；
- remote execution；
- daemon scheduling policy。

这些能力的后续设计必须遵守本文的领域、状态、进程和依赖边界。
