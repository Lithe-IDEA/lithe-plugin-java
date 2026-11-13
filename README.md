# Java Support for Lithe

> **Viva 最小 Maven 工作流（本地验证，尚未发布）**：清单已迁到当前契约 0，后端使用 Core
> `65cf82343c5e315aae2e335b2e8f0217b703e89a` 中的 Rust SDK 0.4.0（尚无对应发布标签）。
> 本轮验证含第三方 JAR 依赖的普通 Maven 工程：JDT 补全、Maven 编译、main 运行和断点调试，包含变量读取、继续和退出。
> 需要同时包含 Lithe 客户端的动作注册解码、动作刷新与首次调试加载修复；未修改这些行为的旧 preview 不能直接使用本包。
> 插件只在 Java 文件/调试被使用时激活，不再在打开任何工作区时启动。
> 运行动作等到 JDT 就绪才注册，复用设置会话已有的代际保护；编译报错时拒绝启动旧 class。
> 渐进导入的后续模块还会发送 ProjectsImported / ClasspathUpdated / ProjectsDeleted，插件按事件刷新动作，
> 不把首次 ServiceReady 当成所有模块已导入；旧服务器或工作区代际的迟到通知会被丢弃。
> 当前宿主未提供 `workspace/findFiles`，因此暂不提供控制台堆栈链接，不申请无效的搜索权限。
> 下文旧客户端/API 版本与发布说明是历史背景，不代表当前 Viva 已验收。


[Lithe](https://github.com/1lck/Lithe-IDEA) 的 Java 插件（`io.github.lithe-idea.java-support`）：
打包 Eclipse JDT Language Server、Lombok、java-debug、vscode-java-test 运行器和每个架构一份 Temurin JDK，
为 Java 项目提供补全、诊断、导航、运行/调试入口、测试运行和 Spring Boot / MyBatis 索引。
它是普通插件（vendor 为 `Lithe Community`），插件包不带发布者签名。

> English: The Java plugin for Lithe, a regular plugin (vendor `Lithe Community`) whose package carries no publisher
> signature. It bundles Eclipse JDT LS, Lombok, the java-debug and vscode-java-test runners and a per-architecture
> Temurin JDK, and provides Java language features, run/debug entry points, test runs and Spring Boot / MyBatis
> indexes. The Java rules live in a Rust domain crate and a plugin backend built on the published Lithe Rust plugin
> SDK. Maven and Gradle import settings come from the Maven and Gradle plugins through a function interface that
> Lithe Core relays between plugins.

> **进行中**：本仓库正在从 Lithe 主仓库拆分出来，目标是 Lithe 宿主（Rust Core 和 macOS 应用）不再包含任何 Java 专属逻辑，
> Java 的领域规则全部由本插件提供，宿主只提供通用插件接口。插件现在带一个 Rust 后端：`plugin.json` 声明了 `backend` 2，
> 后端承担 Maven/Gradle 导入设置的中转（见“工作方式”），并解析语言服务器的启动：`plugin.json` 的 `languageServers[0].launch`
> 是 `kind: "backend"`、函数 `java.resolveLaunch`（`serverProcess` 3），不再有 `hostIntegration`；后端在 `backend/src/launch.rs`
> 里定位包内的 Equinox 启动器、按架构的配置目录、Lombok 代理、java-debug 与 java-test 扩展包和随包 JDK，拼出 JDT LS 的命令行、
> 初始化选项和 `language/status` 就绪门。由插件接管运行/调试/测试还在进行，清单见 `MIGRATION.md` 第 6 节和第 10 节。
> 在宿主去掉内置 Java Support（旧 ID `dev.lithe.plugin.java-support`）之前，本仓库发布的版本标为预发布，原因见 `MIGRATION.md` 第 9 节。
> Maven Archetype 生成器、Maven 与 Gradle 的项目模板已经移交 lithe-plugin-maven 和 lithe-plugin-gradle；
> 在这两个插件接手模板之前不要发布本插件的新版本，否则“新建项目”会少掉 Maven 和 Gradle 选项。

## 当前 Viva 验证入口

先用 `scripts/build-package.sh --arch arm64` 与 `scripts/archive-package.sh` 构建真实插件包。
在包含配套修复的 Lithe 检出中，把 `LITHE_TEST_JAVA_PLUGIN_ARCHIVE` 指向该 ZIP，
`LITHE_TEST_MAVEN_PLUGIN_ARCHIVE` 指向兼容当前契约的 Maven ZIP，再运行：

```sh
scripts/desktop/desktop-go.sh test ./internal/workbench \
  -run '^TestJavaPluginMavenWorkflow$' -v -count=1 -timeout=420s
```

该显式集成测试使用独立用户存储和真实安装器，不修改用户已安装的插件；每轮创建新的 Maven 工程，
执行真实 Maven、JDT LS、Java 和 java-debug，断言补全候选、编译产物、第三方 JAR 类路径、输出、断点与局部变量。
退出通知处理完毕后关闭工作区，核对两个已安装插件的全部文件 SHA-256 不变。
运行日志由宿主的统一目录事件接收；测试不以模拟语言服务器或调试器代替它们。
还覆盖再次调试后的主动停止、编译错误阻止旧 class 启动，并直接读取真实 Core 日志事实，
确认三个运行成功各有 INFO 终态、编译失败只有一个 `run.build_failed` ERROR、操作身份保留，
高频动作收集没有成功 INFO。Debug Stop 同时回收 Run 持有的 JVM。

本轮未验收原生日志窗口、JDT/适配器启动失败或启动中取消，以及所有降级/恢复分支；
上述集成结果不能替代这些日志验收。生成器、JUnit/TestNG、Spring/MyBatis、Gradle 也未在本轮执行。

当前仅在 macOS arm64 的无头 Viva 工作台验证，不能视为原生窗口、Windows/Linux、任意复杂依赖、
私有仓库或完整传统 Java EE 验收。Windows 包和长 classpath 尾项仍见
[MIGRATION.md](MIGRATION.md) 与 [#7](https://github.com/Lithe-IDEA/lithe-plugin-java/issues/7)。

### 旧项目兼容性边界

Java 8 运行时与 Java 5/7 源码级别是两回事。实测 JDK 8 的 Maven 编译可以生成 major 49/51 的
class，补全也可用；但包内 ECJ 3.46.100 直接拒绝 `-source 1.5` 与 `-source 1.7`，只接受
Java 8 及以上。因此当前运行前的 JDT 构建会返回 `run.build_failed`，Java 5/7 原配置的 Run/Debug
**未跑通**，不能用已有 class 绕过构建失败，也不能把安全拒绝测试通过写成该项目受支持。

客户端的 `TestJavaPluginLegacyWorkflows` 验证 Java 8+ 的旧布局、跨模块依赖、中文空格路径、
Profile 源码目录、Java 8 JAXB，以及旧 JUnit 的 Maven test goal；
`TestJavaPluginLegacySourceBoundaries` 单独断言 Java 5/7 的当前限制。Eclipse/Ant 布局通过
`.project`、`.classpath` 导入并由 JDT 构建，不代表执行了 `build.xml` 的 Ant target。
JUnit 的 Maven goal 也不代表 Java 插件的每一种行内测试运行器都已验收。

历史项目入口 `TestJavaPluginHistoricalProject` 只修改隔离副本，增加调用真实项目 API 的验证 main。
`LITHE_TEST_JAVA_HISTORICAL_ROOT` 指向固定 commit，`LITHE_TEST_JAVA_HISTORICAL_PROBE` 提供验证表达式。
原配置与显式 `LITHE_TEST_JAVA_HISTORICAL_SOURCE=1.8` 的临时 POM 修改必须分开报告；后者不能证明
原 Java 5 配置可用。Java 8/11 运行时由 `LITHE_TEST_JAVA8_HOME`、`LITHE_TEST_JAVA11_HOME` 提供，测试
会核对 class 目标版本与实际 JVM，不从 JDT 使用的随包 JDK 推断项目运行时。

| 历史源码基线 | 原 Java 5 POM | 仅在副本把 source/target 改为 1.8 |
| --- | --- | --- |
| Commons CLI 1.3.1，`41d3dbf00f3e2041f5e407b9b96d8f048ab388d9` | Maven 编译/补全通过，Run/Debug 被拒绝 | 补全、编译、API 验证 main、断点变量、继续/停止通过，22.13 秒 |
| Joda-Time 2.3，`b0fcbb950326cbb47452670de9e666ca119ef53d` | Maven 编译/补全通过，Run/Debug 被拒绝 | 同上，23.71 秒 |

两者都增加了调用真实项目 API 的验证入口；未声称运行了原项目的完整应用、所有测试或时区数据生成。
下载源码保持只读，未把第三方源码、产物或用户路径提交到客户端。

正常关闭时宿主仍可能通过 `destroyServer` 的预期拒绝重试等待退出，产生多条 WARN；本轮未观察到
进程残留，已有独立重构单 [Lithe-IDEA #1860](https://github.com/1lck/Lithe-IDEA/issues/1860)。

2026-11-13 本地结果：最终 arm64 ZIP 连续三轮上述完整流程通过，分别 18.54、18.23、19.20 秒。
每轮隔离安装和工作区，但 Maven 依赖缓存与系统文件缓存没有清空；这不是首次联网下载或长期压力测试。
初轮插件 371 项 Rust 测试、客户端全量 Go 测试/`vet`、5 项定向竞态回归通过；
扩展测试发现渐进导入动作刷新遗漏，补上事件驱动刷新和服务器代际校验后，Rust 372 项通过，
真实含中文空格路径的跨模块工程连续五轮完整流程通过。修复前的偶发失败保留在验证记录中，不能只报告重跑成功。
小工程导入后的 JDT 进程树 RSS 约 0.8 GiB，是单点观测，不是峰值或客户端总内存。

本轮清单的 API 和能力版本均为 0，插件归属仍为社区普通插件。权限用途如下；保留的测试、生成器、
Spring/MyBatis 等既有能力不属于本轮 Maven 验收范围。

| 权限 | 用途 |
| --- | --- |
| `backend.launch` | 启动包内 Rust 后端 |
| `documents.read` | 读取当前 Java 文档以收集动作 |
| `documents.contentProvider` | 提供 JDT 虚拟源码 |
| `editor.actions` | 注册 Run/Debug 行内动作 |
| `languageServer.launch` | 启动包内 JDT LS |
| `languageServer.request` | JDT 构建、main、测试、类路径和调试扩展请求 |
| `project.generate` | 既有 Java/Spring Boot 生成器 |
| `debug.launch`、`debug.attach`、`debug.start` | 声明 java-debug 适配器并经 Run 工作流发起调试 |
| `run.start` | 启动用户项目或测试 JVM |
| `window.messages` | 显示测试结果摘要 |
| `workspace.metadata` | 取得工作区上下文与宿主提供的状态位置 |

参考资料为上述 Core commit 的 `shared/plugins/contract/v1/` 与 Rust SDK 0.4.0。
该 commit 的 SDK README/日志指南写有 `sdk-rust-v0.4.0` 发布标签，但远端当时没有这个标签或资产，
因此依赖固定到同一 Core commit，不把未发布标签当作可下载输入。

## 功能

| 功能 | 说明 |
| --- | --- |
| 语言服务 | 用打包的 JDT LS（含 Lombok）提供补全、诊断、导航；JDK 随包提供，只用来运行语言服务器，项目自己的 SDK 仍由用户选择 |
| 运行与调试 | 在 `main` 方法和测试旁显示 Run/Debug 行内动作；点击后后端经 JDT LS 构建并解析类路径，用用户选择的 JDK 通过宿主 Run 窗口启动，调试经 java-debug 连接；当前 Viva 包暂不提供控制台堆栈链接 |
| 测试 | 发现 JUnit/TestNG 测试项，按类或方法启动 Eclipse JUnit 运行器（TestNG 用 vscode-java-test 运行器），运行结束后显示通过/失败计数 |
| Spring Boot / MyBatis | 索引 Spring 端点与 MyBatis 映射，支持跳转 |
| 新建项目 | 两个声明式项目生成器：Java（无构建系统，可选示例代码）、Spring Boot；Maven 原型和 Maven/Gradle 项目模板由 Maven、Gradle 插件提供 |
| Maven / Gradle 工程 | 导入设置（settings 文件、本地仓库、offline、Gradle user home 等）由 Maven、Gradle 插件通过 `languageImport.settings` 提供，Java 不计算也不解析它们；后端收下后推给 JDT LS，Maven profiles 变化时触发 `java.project.updateSettings` |

## 工作方式

- **领域规则**在 `crates/java-domain`，不依赖宿主：命令的 JSON 载荷和错误码与主仓库 Core 一一对应。
  所有宿主差异（存储、Git、语言服务器通道）都是带 `TODO(host-api: API-NN)` 的占位；日志已接入宿主统一日志，见“日志”一节。
- **插件后端**在 `backend/`：基于固定版本的 Rust SDK（`sdk.lock.json`），把每条领域命令注册为同名插件函数，
  并带着跨插件函数的调用方与导出方两侧。`plugin.json` 用 `backend.launch`（`kind: "packaged"`，路径 `bin/lithe-java-backend`）声明它，
  打包脚本把按架构编译好的二进制放进包的 `bin/`。后端是全机一个进程，所有工作区共享。
- **设置中转**（Maven/Gradle 导入设置到 JDT LS）：
  - 调用方向：宿主通知 `lithe/host/languageServer/didChangeState` 说某个工作区的 Java 语言服务器进入 `running` 后，后端通过 Core 中转发现所有导出
    `languageImport.settings` 的插件（Maven、Gradle），逐个调用，只接受落在 JDT LS 导入设置命名空间里的键，合并后用
    `languageServer/notify` 向语言服务器推送 `workspace/didChangeConfiguration`。语言服务器重启（代号变化）后会重新收集。
  - 导出方向：Maven/Gradle 设置变化时调用 Java 导出的 `languageImport.settingsChanged`（`context: "workspace"`）。后端按 Core 写入的调用方插件 ID
    **整份替换**该调用方自己的槽位，不会影响别的提供方；调用里可以附带 `projects`（`{projectUri, selectedProfiles}`），
    后端据此向 JDT LS 发 `java.project.updateSettings`，应用 Maven profiles。Java 不会反过来调用正在调用它的插件，否则调用链会被 Core 当作环路拒绝。
  - 过渡规则：没有任何提供方时后端什么都不推送，主仓库 jdt 宿主集成推送的设置保持生效；后端的整份替换只在至少有一个提供方参与后才发生。
  - 清单声明：`requires.backend: 2`、`pluginFunctions: 1`、`languageServers: 3`；`pluginFunctions.calls` 为 `languageImport.settings`，
    `exports` 为 `languageImport.settingsChanged`；权限 `backend.launch`、`languageServer.request`、`workspace.metadata`；
    语言服务器声明 `requestMethods: ["workspace/executeCommand"]` 和 `commands: ["java.project.updateSettings"]`；激活事件 `onWorkspaceOpen`，
    这样打开工作区时后端就会启动，Maven/Gradle 才能随时调用它。
  - 当前不推送 `java.configuration.runtimes`：插件后端还没有列出 JDK 安装的宿主方法，等 API-23（`toolchains.list`）提供。JDT LS 不会因为缺这个键而删除已登记的 VM。

  细节见 `MIGRATION.md` 第 5 节。
- **运行/调试/测试**在 `backend/src/run.rs`：用宿主的编辑器动作（`editor/registerActions`）提供行内 Run/Debug 标记，
  用 `run/start` 启动进程（进程用清单 `requiresTools: ["java"]` 引用的 JDK 工具定义），测试结果由后端自己在回环端口上监听
  JUnit 运行器并用窗口消息显示，输出链接匹配器把堆栈行解析成工作区文件。细节见 `MIGRATION.md` 第 5.9 节。
- **运行时只读**：插件包安装后在运行时只读。JDT LS、Lombok、java-debug、测试运行器、JDK 和后端二进制都在构建阶段固定来源、校验哈希（或由锁定的源码编译）后放进包里，
  运行时状态只写宿主提供的状态目录，不写包目录。

## 日志

后端把日志交给宿主的统一日志 `litheLogging` 0，不写 stderr。函数 ID 都是编译期常量，注册只会因 ID 写错、重复或超出 SDK
上限而失败，单元测试 `the_backend_builds_with_every_command_registered` 在 CI 里挡住这种构建；万一发生，进程在握手前以状态 1 退出，
由宿主记录启动失败（`backend/src/main.rs`）。当前固定的 Rust SDK 0.4.0 要求宿主提供 `litheLogging` 0，
不提供的 Lithe 版本会拒绝启动后端；本次只验证上述配套 Viva 检出，尚未发布。

| 事实 | 级别 | 来源 | 内容 |
| --- | --- | --- | --- |
| 关键操作成功：`runConfig.selectJava`、`debug.javaTestLaunch`、`execution.planLaunchCommand`、`java.prepareLaunch`、`java.resolveLaunch`、`java.runAction`、`java.rerun`、`languageImport.settingsChanged`（`backend/src/lib.rs` 的 `KEY_OPERATIONS`） | INFO | SDK 自动 | 操作 ID、耗时 |
| 其余函数成功（编辑器驱动或启动时的读取，如 `java.runMarkers`、`java.codeVision`、`spring.index`） | 不记录 | `suppress_success_diagnostics`（`backend/src/lib.rs`） | — |
| 任一函数失败 | ERROR | SDK 自动 | 类别为 `pluginErrorCode`（如 `java.invalidRequest`；`languageImport.settingsChanged` 的 `relay.caller_missing`、`relay.context_missing`、`relay.invalid_params`）、耗时；不含消息与 `details` |
| 领域命令失败类别 | ERROR | `plugin_error_code` | `java.invalidRequest`、`java.workspaceNotFound`、`java.permissionDenied`、`java.notSupported`、`java.runtimeMissing`、`java.processStartFailed`、`java.processFailed`、`java.parseFailed`、`java.cancelled`、`java.timedOut`、`java.connectionClosed`、`java.unknown`；SDK 识别的取消不记录 |
| JDT 启动规划失败 | ERROR | SDK 自动 | `launch.invalid_params`、`launch.package_unavailable`、`launch.resource_missing`、`launch.configuration_failed` |
| JDT 配置清理或工作区读取降级 | WARN | SDK `degraded` | `launch.configuration_cleanup_failed`、`launch.workspace_folders_unavailable` |
| 取消 | 不记录 | SDK | — |
| 列出 `languageImport.settings` 提供方失败、单个提供方失败或超时、答复不可用、答复含越界键、提供方导出的函数版本不符 | WARN（`degraded`） | `backend/src/plugin_functions.rs` | 类别 `relay.providers_unavailable`、`relay.provider_failed`、`relay.provider_timed_out`、`relay.settings_rejected`、`relay.keys_rejected`、`relay.provider_version_mismatch`，计数；不含提供方 ID |
| 取不到工作区根目录、设置推给 JDT LS 失败、单个工程的 profiles 更新失败、语言服务器状态通知格式不对 | WARN（`degraded`） | `backend/src/settings_session.rs` | 类别 `session.root_unavailable`、`session.push_failed`、`session.project_update_failed`、`session.state_invalid`，计数；操作 ID 为 `notification`（状态通知触发）或 `languageImport.settingsChanged` |
| 插件存储清理失败 | WARN（`degraded`） | `crates/java-domain/src/host/diagnostics.rs`（`host::diagnostics`） | 类别 `storage.cleanup_failed.<阶段>`（阶段为 `jdt-configuration`、`jdt-settings`、`java-argfile-remove`），计数 |
| 运行动作失败 | ERROR | SDK 自动 | `backend/src/run.rs` 的 `RUN_ERROR_CODES`：`run.invalid_params`、`run.context_missing`、`run.workspace_unavailable`、`run.language_server_failed`、`run.build_failed`、`run.no_tests`、`run.port_unavailable`、`run.host_rejected` |
| 动作注册、上下文读取或测试结果摘要降级 | WARN | SDK `degraded` | `RUN_CATEGORIES`：`run.actions_registration_failed`、`run.contexts_unavailable`、`run.result_summary_failed` |
| 宿主启动 JDT LS、java-debug、项目 JVM | 宿主决定 | 声明式贡献/Run 生命周期 | 插件不重复发送启动终态；上游 stderr 由宿主按 DEBUG 接收 |
| JDT LS 自身日志 | 上游，默认 WARNING | `plugin.json` 的 `-Dosgi.logfile=${logDir}/jdt.log` | 只写宿主提供的会话日志目录，单文件 1 MB、保留 2 个备份；用户可在设置里调 `logLevel` |

日志事实只带类别、计数和耗时，不含路径、源码、工具输出或用户内容。`${logDir}` 由宿主经可写存储适配器提供，不写插件包或 app bundle。
Rust 后端的通道是 `plugins/io.github.lithe-idea.java-support/host`；JDT LS、java-debug 与项目 JVM
分别由宿主的语言服务器、调试和 Run 通道管理。只有 JDT LS 使用原生日志文件。

## 安装

Lithe 不再区分官方插件和普通插件，所有插件都按普通插件处理：本插件不带发布者签名（`plugin.json` 的 `vendor.signatureRequirement` 为 `none`）。
区别只在 [Lithe Store](https://github.com/Lithe-IDEA/Lithe-store) 怎样收录：Lithe-IDEA 组织里 `lithe-plugin-*` 仓库的稳定版（非预发布）
GitHub Release 由商店机器人自动跟踪并上架；组织之外的插件向 Lithe-store 提交 pull request，维护者审核后上架。

目前本仓库只发布预发布版，商店机器人会跳过，所以还不能从 Lithe Store 安装。当前版本的 Lithe 也会拒绝或无法完整运行这个包：
本包声明了 `backend` 2、`languageServers` 3（插件自有语言服务器会话）和 `pluginFunctions` 1，宿主要包含主仓库的
[#1745](https://github.com/1lck/Lithe-IDEA/pull/1745)、[#1746](https://github.com/1lck/Lithe-IDEA/pull/1746) 之后的预览版才提供它们，稳定版还没有；
此外 Lithe 仍然内置旧 ID `dev.lithe.plugin.java-support` 的 Java Support。
（普通插件使用 JDT 宿主集成和短工具 ID `java` 的限制已由主仓库 [#1763](https://github.com/1lck/Lithe-IDEA/pull/1763) 取消。）
具体原因和转为正式发布的条件见 `MIGRATION.md` 第 9 节。下载的 ZIP 通过 **设置 → 插件** 的 ⚙ 菜单 → **从磁盘安装插件…** 安装。

## 开发

```text
io.github.lithe-idea.java-support/   # 插件包的静态部分：plugin.json、icons/、templates/，目录名必须等于插件 id（包里的 bin/ 由打包脚本编译后端后生成）
crates/java-domain/              # Java 领域规则（Rust），测试在各模块内和 tests/ 下
backend/                         # 插件后端进程（lithe-java-backend），基于 Lithe Rust 插件 SDK
fixtures/                        # 与主仓库共享的契约样例（JSON）
third_party/                     # JDT LS 与 JDK 的来源地址和 SHA-256
sdk.lock.json                    # 后端链接的 Rust SDK：标签、提交、crate 归档的 SHA-256
scripts/                         # 组装、校验、归档插件包，检查 SDK 锁
.github/workflows/ci.yml         # PR：格式、lint、测试、SDK 锁，以及 macOS 上的打包任务（构建并归档两个架构）
.github/workflows/release.yml    # 推送 v<版本> 标签：测试后打包并发布 GitHub Release（预发布）
MIGRATION.md                     # 主仓库路径到本仓库的映射、待宿主接口清单、主仓库删除清单
```

Rust 部分需要 Rust 工具链和 Cargo；打包脚本需要 macOS、`zsh`、`curl`、`ditto`，不需要 Xcode 或 Swift 工具链。
打包脚本会用 `cargo build --release --locked -p lithe-java-backend --target <triple>` 编译后端，所以还要安装两个 Apple target：
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`（`--arch arm64` 用前者，`x86_64` 用后者，`universal` 两个都编译再用 `lipo -create` 合并）。

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/verify-sdk-lock.sh
```

所有测试都是有界、确定性的：不联网，不读全局时间，不占固定端口。

构建并归档插件包（会下载并校验 JDT LS 与 JDK，体积较大）。本地开发、CI 和发布使用同一组命令，产出同一种归档：

```bash
scripts/build-package.sh --arch arm64          # 或 x86_64；打印包目录 .artifacts/package/<arch>/io.github.lithe-idea.java-support
scripts/archive-package.sh --package <上一步打印的目录> --arch arm64   # 写入 dist/Lithe-Java-Support-<版本>-darwin-{arm64,x64}.zip 和对应的 .sha256
```

`build-package.sh` 会在输出包目录前校验它；`archive-package.sh` 把归档解开后再校验一次。单独校验一个包目录用
`scripts/verify-package.sh --package <目录> --arch arm64|x86_64|universal`：它要求 `plugin.json` 的 `vendor.signatureRequirement` 为 `none`，
包里不能有 `lithe-plugin-signature.json`，`bin/` 里只能有 `backend.launch.path` 指向的后端二进制，它必须可执行且包含请求的架构（`lipo -archs`），
并检查打包的 JDK 保留着上游的 Apple 代码签名。归档不需要签名密钥，也不调用任何签名工具。

`plugin.json` 的完整格式见 lithe-plugin-docs 里的
[插件契约](https://github.com/Lithe-IDEA/lithe-plugin-docs/blob/main/contract/v1/README.md)。

## 发布新版本

1. 修改 `io.github.lithe-idea.java-support/plugin.json` 的 `version`，合入 `main`。
2. 推送同名标签（带 `v` 前缀，必须等于清单版本）：

   ```bash
   git tag v1.2.0 && git push origin v1.2.0
   ```

3. `Release` 工作流先在 Ubuntu 上跑 Rust 检查，再在 macOS 上构建、校验并归档 arm64 与 x86_64 两个包，然后发布。
   发布资产是 `Lithe-Java-Support-<版本>-darwin-{arm64,x64}.zip` 和对应的 `.sha256`。没有任何步骤读取签名密钥，包也不带发布者签名。
4. 发布保持为预发布（`--prerelease`）：Lithe Store 的自动跟踪会跳过预发布，原因有三个。一是目前的 Lithe 仍然内置旧 ID 的 Java Support；
   二是包声明了 `backend` 2、`languageServers` 3、`pluginFunctions` 1，后端链接的 Rust SDK 0.3.0（插件函数协议 4）还要求宿主提供 `litheLogging` 1，
   需要包含主仓库 #1745、#1746 的预览版，稳定版还没有；
   三是 Maven 与 Gradle 的新建项目模板已经移交 lithe-plugin-maven 和 lithe-plugin-gradle，这两个插件接手之前发布新版本，
   “新建项目”会少掉 Maven 和 Gradle 选项，所以**在它们接手模板之前不要发布本插件的新版本**。
   等 Lithe 主仓库的阶段 E 完成（`MIGRATION.md` 第 8.2 节）、Lithe 稳定版提供上述接口、两个构建插件发布后，去掉 `--prerelease`，
   商店就会自动上架稳定版。细节见 `MIGRATION.md` 第 9 节。

## 许可

插件自身的代码按 Apache License 2.0 发布（见 `LICENSE`）。插件包内的第三方组件各自保留原许可，见 `NOTICE`。
