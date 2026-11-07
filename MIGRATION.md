# 迁移说明：Java 逻辑从 Lithe 主仓库移到本仓库

本文是 Java Support 插件（`io.github.lithe-idea.java-support`）从 Lithe 主仓库拆分出来的移植账本，回答四个问题：

1. 主仓库里的每个 Java 相关文件或符号，现在在本仓库的哪里，状态是什么。
2. 哪些部分在等宿主（Lithe Core 与 macOS 应用）提供通用插件 API，等的是计划里的哪个编号。
3. Maven 和 Gradle 的导入逻辑为什么、怎样交给 Maven/Gradle 插件。
4. 主仓库最终要删什么，按什么顺序删。

> English: This is the porting ledger for splitting the Java Support plugin out of the Lithe main repository. It maps
> every Java-related source file of Lithe (`path@1daaeb525`) to its new location with exactly one status, lists what
> waits for generic host APIs (tagged with the plan number from `docs/development/macos-plugin-api-plan.md`), records
> that Maven/Gradle import logic belongs to the Maven/Gradle plugins, and gives the phased deletion list for the
> main repository. Paragraph text is Chinese; identifiers, paths and API numbers are literal.

## 1. 概览

- 来源快照：Lithe `origin/preview` 的提交 `1daaeb525`。表里的路径都写成 `路径@1daaeb525`，只读，不在本仓库改动。
- 本仓库产物：
  - Rust 工作区共约 23,500 行 Rust：`crates/java-domain`（领域规则，不依赖宿主）和 `backend`（基于已固定版本 SDK 的插件后端进程）。
  - 344 个测试：领域 crate 281 个，后端 63 个（单元 54 个、`repository_pins` 6 个、`session` 3 个）。全部是有界、确定性的：
    不联网，不读全局时间，不占固定端口。其中设置中转相关的测试（`jdt_contributions.rs`、`plugin_functions.rs`、`settings_session.rs`）
    用桩实现验证收集、替换和推送逻辑，不连宿主，见第 10 节。
  - 18 个 fixture（共 1,234 行）放在 `fixtures/`。其中 12 个被 Rust 测试读取；另外 6 个是从主仓库复制来的契约样例，
    暂时没有 Rust 测试读取（见第 7 节）。
  - 插件包：`io.github.lithe-idea.java-support/`，加上组装、校验、归档脚本、CI 和发布工作流。
- 领域代码的注释（`//!`、`///`）全部是英文。每个来自 Core 或 Swift 的模块在模块文档里写明来源。
- 宿主侧的接线是分步的：`plugin.json` 现在声明 `backend`（`backend.launch` 为 `packaged`，路径 `bin/lithe-java-backend`），后端把领域命令注册成插件函数，
  并承担 API-35 的设置中转（第 5 节：调用方一侧发现并调用 `languageImport.settings`，导出方一侧接收 `languageImport.settingsChanged`，
  再用 `languageServer/notify` 推送给 JDT LS）。语言服务器由后端启动（`launch.kind` 为 `backend`，函数 `java.resolveLaunch`，
  `requires.serverProcess` 3，不再有 `hostIntegration`），见第 5.8 节；运行/调试/测试的接管见第 10 节。

## 2. 边界决定

### 2.1 Maven 和 Gradle 的导入逻辑整块归 Maven/Gradle 插件

纯 Java 项目没有"导入工程"这一步，JDT LS 直接按源码目录工作。导入只出现在 Maven 或 Gradle 项目里，所以：

- settings 文件、本地仓库、profiles、Gradle offline、gradleUserHome、从 POM 读出的源码目录，全部由 Maven/Gradle 插件计算。
- Java 插件不解析 `settings.xml`，不处理 profiles，不读 POM，也不写任何 Maven/Gradle 状态目录。
- Java 插件只声明"我的 JDT LS 接受别的插件通过 settings 片段给它哪些键"，见 `CONTRIBUTED_SETTINGS_KEYS`
  （`crates/java-domain/src/lsp/languages/jdt.rs`），并校验收到的片段不越界（`accept_contributed_settings`）。
  这是 JDT LS 自己的设置名空间，不是 Maven 逻辑。

已经从移植结果里删掉、交给 Maven/Gradle 插件的内容见第 4 节。

### 2.2 插件之间只经 Core 中转的跨插件函数通信（API-35）

插件 1 → Core → 插件 2：

- 接收方在清单里显式导出函数，调用方声明它要调用的函数 ID（接口名），不写对方的插件 ID。
- Java 插件定义接口函数 ID（例如 `languageImport.settings`），Maven 和 Gradle 插件各自导出同一个 ID。
- Java 的语言服务器进入运行状态（宿主通知 `lithe/host/languageServer/didChangeState`）后，Java 的后端向宿主发现所有已启用且导出该 ID 的插件，
  逐个调用，合并结果，用 `languageServer/notify` 以 `workspace/didChangeConfiguration` 交给 JDT LS。
- Maven 设置变化时，Maven 插件反过来调用 Java 导出的"变化通知"函数，并把新值放在调用参数里。Java 不在这次调用里回调 Maven，
  否则调用链 A→B→A 会被 Core 当作环路拒绝。
- 信任不传递：被调方用自己的权限执行，调用方不获得任何权限。

中转本身已由主仓库 [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 在 Core 里实现并合入。本仓库实现了 Java 一侧：接口常量、答案校验与合并（`jdt_contributions.rs`），
发现并调用提供方的后端模块（`plugin_functions.rs`，集中了全部中转协议细节，见第 5 节），以及按工作区维护贡献并推送给 JDT LS 的
`settings_session.rs`。后端链接支持插件函数协议 4 的 Rust SDK 0.3.0，按已合入的方法形状调用中转；`plugin.json` 已声明 `backend` 和 `pluginFunctions`（第 5.7 节）。

### 2.3 计划编号约定

每个"等待宿主 API"的条目都标注 `docs/development/macos-plugin-api-plan.md` 里的编号，不新增编号。本任务指定的编号是：

| 编号 | 范围 |
| --- | --- |
| API-25 | 语言服务器 |
| API-27 | 编辑器动作与标记 |
| API-29 | 构建与测试 |
| API-33 | 项目模板 |
| API-35 | 跨插件函数 |

少数条目最贴合的是计划里已有、但不在上面五个之内的编号：API-20（存储）、API-22（日志与进度）、API-23（工具链与进程）、
API-30（调试适配器）、API-37（环境与 l10n）、API-42（内置 Git 只读）。这些条目在表里的编号后面加 `†`，
意思是"编号来自计划，但不在任务指定的五个之内，请协调者确认归类"。没有自造任何新编号。

## 3. 路径映射表

状态只有四种，每行一种：**已移植并有测试**、**已移植无测试**、**等待宿主 API（编号）**、**不移植（原因）**。
"新位置"中的路径都相对本仓库根目录；`dj` 表示 `crates/java-domain/src`。括号里的数字是该文件里的测试数量。

### 3.1 Rust Core 源码

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `rust/lithe-core/src/lsp/languages/java_entrypoints.rs@1daaeb525` | `dj/lsp/languages/java_entrypoints.rs` | 已移植并有测试 | 10 个测试 |
| `…/lsp/languages/java_launch.rs@1daaeb525` | `dj/lsp/languages/java_launch.rs` | 已移植并有测试 | 2 个测试，读 `java-launch-preparation-v1.json` |
| `…/lsp/languages/java_main_methods.rs@1daaeb525` | `dj/lsp/languages/java_main_methods.rs` | 已移植并有测试 | 5 个测试 |
| `…/lsp/languages/java_navigation_syntax.rs@1daaeb525` | `dj/lsp/languages/java_navigation_syntax.rs` | 已移植并有测试 | 3 个测试 |
| `…/lsp/languages/java_run_markers.rs@1daaeb525` | `dj/lsp/languages/java_run_markers.rs` | 已移植并有测试 | 8 个测试，含 Swift `JavaRunMarkerTests` 的场景 |
| `…/lsp/languages/java_tests.rs@1daaeb525` | `dj/lsp/languages/java_tests.rs` | 已移植并有测试 | 5 个测试 |
| `…/lsp/languages/java_workspace.rs@1daaeb525` | `dj/lsp/languages/java_workspace.rs` | 已移植并有测试 | 7 个测试：工作区策略、缓存保留、指纹 |
| `…/lsp/languages/jdt.rs@1daaeb525` | `dj/lsp/languages/jdt.rs` | 已移植并有测试 | 31 个测试。Maven/Gradle 部分已删，换成贡献设置模型（第 4、5 节） |
| `…/lsp/languages/jdt_build.rs@1daaeb525` | `dj/lsp/languages/jdt_build.rs` | 已移植并有测试 | 14 个测试。Maven profiles 阻塞项已删，换成 `ContributedSettings` 阻塞项 |
| `…/lsp/languages/jdt_configuration.rs@1daaeb525` | `dj/lsp/languages/jdt_configuration.rs` | 已移植并有测试 | 7 个测试 |
| `…/lsp/languages/jdt_navigation.rs@1daaeb525` | `dj/lsp/languages/jdt_navigation.rs` | 已移植并有测试 | 3 个测试 |
| `…/lsp/languages/jdt_progress.rs@1daaeb525` | `dj/lsp/languages/jdt_progress.rs` | 已移植并有测试 | 5 个测试，读 `jdt-readiness-v1.json` |
| `…/lsp/languages/jdt_project_metadata.rs@1daaeb525` | `dj/lsp/languages/jdt_project_metadata.rs` | 已移植并有测试 | 6 个测试。"哪些文件被 Git 忽略"的查询是占位，见第 6 节 API-42† |
| `…/lsp/languages/project_preparation.rs@1daaeb525` | `dj/lsp/languages/project_preparation.rs` | 已移植并有测试 | 3 个测试。Maven profile 阶段已删 |
| `…/lsp/languages/mod.rs@1daaeb525` | `dj/lsp/languages/mod.rs` | 已移植无测试 | 只做模块注册，只保留 Java 模块 |
| `…/lsp/languages/jdt_maven_settings.rs@1daaeb525` | 无（交付目录，见第 4 节） | 不移植（Maven 插件） | 生成 JDT 用的 Maven settings；12 个测试随文件交给 Maven 插件 |
| `…/lsp/languages/{catalog,indexing,swift}.rs@1daaeb525` | 无 | 不移植（通用语言目录与 Swift，不是 Java 逻辑） | 留在 Core |
| `…/lsp/mod.rs@1daaeb525`（生命周期类型） | `dj/lsp/mod.rs` | 已移植无测试 | 复制 `LspLifecycleState`、`JavaRuntimeCandidate`、`LspRuntimeError`，只服务 Java 模块 |
| `…/lsp/interface/engine.rs@1daaeb525`：`java_navigation_markers`、`java_resolve_navigation`、`java_prepare_launch`、`prepare_java_launch`、`dispatch_java_entrypoints_locked`、`dispatch_java_build_locked`、`complete_java_build_locked`、`java_executable_from_environment`、就绪门、`StartServerRequest` 的 Java 字段 | 无；命令名在 `dj/commands.rs` 里占位（`SESSION_COMMANDS`，调用返回 `NotSupported`） | 等待宿主 API（API-25） | 这些函数驱动正在运行的 JDT LS 会话，需要宿主把"向声明的语言服务器发请求并收通知"的通道开放给插件后端 |
| `…/lsp/interface/engine.rs@1daaeb525`：`retry_maven_profiles`、`update_maven_configuration`、`maven_profile_requests`、`push_maven_profile_*` | 无 | 不移植（Maven 插件，经 API-35 通知 Java） | 见第 4、5 节 |
| `…/lsp/interface/engine_real_jdt_tests.rs@1daaeb525` | 无 | 等待宿主 API（API-25） | 真实 JDT 端到端测试依赖宿主会话；API-25 通道就绪后在本仓库重做为 e2e |
| `…/lsp/interface/engine_real_jdt_maven_provider_tests.rs@1daaeb525` | 无 | 不移植（Maven 插件） | 依赖 Maven provider，归 Maven 插件的 e2e |
| `…/languages/java.rs@1daaeb525` | `dj/languages/java.rs` | 已移植并有测试 | 6 个测试，另有 `tests/java_commands.rs` |
| `…/languages/java_syntax.rs@1daaeb525` | `dj/languages/java_syntax.rs` | 已移植并有测试 | 3 个测试 |
| `…/languages/spring.rs@1daaeb525` | `dj/languages/spring.rs` | 已移植并有测试 | 17 个测试，另有 `tests/spring.rs` |
| `…/languages/mybatis.rs@1daaeb525` | `dj/languages/mybatis.rs` | 已移植并有测试 | 文件内没有单元测试，由 `tests/mybatis.rs` 的 10 个测试覆盖 |
| `…/languages/text.rs@1daaeb525` | `dj/languages/text.rs` | 已移植并有测试 | 2 个测试；从 Core 复制，Java 模块要用的文本工具 |
| `…/debug/java_test.rs@1daaeb525` | `dj/debug/java_test.rs` | 已移植并有测试 | 3 个测试，读 `java-test-launch-v1.json` |
| `…/debug/engine.rs@1daaeb525`、`debug/types.rs`、`debug/adapters.rs` 里的 Java 特例（RedefineClasses、step filters、DevTools、vmArgs、类名过滤、`SELECTED_RUNTIME_TOOLS`） | 无 | 等待宿主 API（API-30†） | 通用化为调试适配器声明的 `features`/`defaults`，Java 插件在 `plugin.json` 里声明 |
| `…/execution/java_selection.rs@1daaeb525` | `dj/execution/java_selection.rs` | 已移植并有测试 | 文件内没有单元测试，由 `tests/java_selection.rs` 的 4 个测试覆盖 |
| `…/execution/configuration.rs@1daaeb525`：`version_parts`、`version_satisfies`、`read_requirements`、`java_feature_version_parts` | `dj/execution/toolchain_requirements.rs` | 已移植无测试 | 复制 `java_selection` 依赖的几个纯函数，只由 `java_selection` 的测试间接覆盖。Maven/Gradle 要求由它们自己的插件提供，见第 6 节 |
| `…/execution/configuration.rs@1daaeb525` 其余 Java 类型（`javaMain`/`springBoot` 到 `java.main` 的 provider 映射、`java-main:` ID、`toolchain.java.homePath`、`extensions["java"]`） | 无 | 等待宿主 API（API-29） | 运行配置里 Java 类型的字段需要由插件声明、宿主通用编辑器渲染 |
| `…/execution/launch_command.rs@1daaeb525` | `dj/execution/launch_command.rs` | 已移植并有测试 | 11 个测试 |
| `…/execution/classpath_jar.rs@1daaeb525` | `dj/execution/classpath_jar.rs` | 已移植并有测试 | 8 个测试 |
| `…/execution/detectors/{maven,gradle}.rs@1daaeb525` | 无 | 不移植（Maven/Gradle 插件，声明式 `buildSystems.detection`） | |
| `…/execution/workspace_scan.rs@1daaeb525`、`project/indexing.rs@1daaeb525` 里的 Java 分支 | 无；`dj/host/indexing.rs` 只有占位 | 等待宿主 API（API-25） | 索引排除规则必须由宿主交给语言服务器的所有配置通道，插件没有这个通道 |
| `…/project/{maven,maven_dependency_tree,maven_test_reports}.rs@1daaeb525` | 无 | 不移植（Maven 插件） | Windows 仍在用，见第 8 节 |
| `…/build/language_import.rs@1daaeb525`、`build/integrations/{mod,maven}.rs@1daaeb525` | 无 | 不移植（Maven/Gradle 插件，被 API-35 取代） | 见第 4 节 |
| `…/build/language_import.rs@1daaeb525` 的调用方一侧（启动 JDT LS 前收集各构建系统的导入设置） | `dj/lsp/languages/jdt_contributions.rs`（新写） | 已移植并有测试 | 11 个测试。接口常量 `languageImport.settings` 与 `languageImport.settingsChanged`、答案校验、按提供方分槽的合并与状态；不含任何 Maven/Gradle 逻辑，见第 5 节 |
| 同上（跨进程调用部分） | `backend/src/plugin_functions.rs`（新写） | 已移植并有测试 | 经 Core 中转发现并调用提供方（`lithe/host/functions/providers`、`lithe/host/functions/invoke`），按已合入的 [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 协议实现，用桩中转测试，见第 5.7 节 |
| `…/lsp/interface/engine.rs@1daaeb525`：`update_maven_configuration` 的接收一侧（Maven 设置变化后通知 Java） | `languageImport.settingsChanged` 的常量与分槽逻辑在 `jdt_contributions.rs`；导出函数与会话触发在 `backend/src/lib.rs`、`backend/src/settings_session.rs` | 已实现，宿主侧接线随阶段 J2 | 中转在 Lithe [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 合入；调用方插件 ID 在插件函数协议 4 的 `caller` 字段里，由 Rust SDK 0.3.0 经 `Context::caller()` 提供，按它分槽，调用方无法伪造，见第 5.7 节 |
| `…/plugins/declarative.rs@1daaeb525`、`build/{invocation,manifest,detect,plans}.rs@1daaeb525` 里的 Java 特判（`KNOWN_RUNTIME_TOOLS*`、`HOST_INTEGRATION_*`、`ModelSource::HostIntegration`） | 无 | 不移植（宿主通用代码里的 Java 特判，不是插件逻辑） | 在主仓库删，见第 8 节 |
| `…/protocol/cancellation.rs@1daaeb525` | `dj/protocol/cancellation.rs` | 已移植并有测试 | 8 个测试 |
| `…/protocol/contracts.rs@1daaeb525`（Java、Spring、MyBatis 模型） | `dj/protocol/contracts.rs` | 已移植无测试 | 只复制 Java 相关请求与响应类型，由各命令的测试间接覆盖；`Maven*Response` 不复制 |
| `…/protocol/error.rs@1daaeb525`（`CoreError`、`ErrorCode`） | `dj/protocol/error.rs` | 已移植无测试 | 保持错误码与 Core 一一对应，`backend/src/lib.rs` 把它们映射到稳定的 `pluginErrorCode` |
| `…/protocol/command.rs@1daaeb525`、`runtime/dispatcher.rs@1daaeb525` 里的 Java 命令（`java.*`、`spring.index`、`mybatis.index`、`lsp.jdtWorkspaceKey`、`runConfig.selectJava`、`debug.javaTestLaunch`） | `dj/commands.rs`（`execute`、`cancel`）；`backend/src/lib.rs` 把每条命令注册成同名插件函数 | 已移植并有测试 | `commands.rs` 4 个测试，后端 `session.rs` 与单元测试 3 个。命令名、载荷、响应和 `InvalidRequest` 消息与 Core 一致，宿主切换调用方时不改 JSON |
| `…/protocol/command.rs@1daaeb525`：`java.navigationMarkers`、`java.resolveNavigation`、`java.prepareLaunch` | `dj/commands.rs`（`SESSION_COMMANDS`，答 `NotSupported`） | 等待宿主 API（API-25） | 同 `engine.rs` 那一行 |
| `…/protocol/command.rs@1daaeb525`：`maven.*`、`lsp.retryMavenProfiles`、`lsp.updateMavenConfiguration`、`build.languageImport` | 无 | 不移植（Maven/Gradle 插件） | |
| `…/lib.rs@1daaeb525`：公开函数 `jdt_workspace_key` | `lsp.jdtWorkspaceKey` 命令（`dj/commands.rs`） | 已移植并有测试 | `tests/java_commands.rs` 里读 `jdt-workspace-key-v1.json` |
| `…/lib.rs@1daaeb525`：公开函数 `settings_with_local_repository` | 无（交付目录，见第 4 节） | 不移植（Maven 插件） | |
| `…/diagnostics/catalog_generated.rs@1daaeb525` 里的 Java 日志事件 | 后端经 Rust SDK（0.2.0 起，当前 0.3.0）接入宿主统一日志 `litheLogging` 1：`backend/src/lib.rs`（`KEY_OPERATIONS` 与成功抑制）、`backend/src/plugin_functions.rs`（中转降级类别）、`dj/host/diagnostics.rs`（清理失败） | 已移植并有测试 | 删除了 `Context::log` 和所有 stderr 日志。级别、类别和内容见 README 的“日志”一节；SDK 要求宿主提供 `litheLogging` 1，见第 9.2、9.3 节 |
| Swift `MacJavaLaunchArgumentPreparer` 的规则部分（Core 里没有） | `dj/execution/argfile.rs` | 已移植并有测试 | 4 个测试：只在命令过长时写 argfile，仅属主可读，租约释放时删除 |

### 3.2 Rust Core 测试与 fixture

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `rust/lithe-core/src/tests/languages.rs@1daaeb525`（Java 部分） | `dj/tests/java_commands.rs` | 已移植并有测试 | 14 个测试 |
| `…/tests/run_configuration.rs@1daaeb525`（JDK 选择部分） | `dj/tests/java_selection.rs` | 已移植并有测试 | 4 个测试 |
| `…/tests/run_configuration.rs@1daaeb525` 其余（约 6,100 行，651 处 Java/Maven 命中） | 无 | 不移植（通用运行配置测试，留在 Core；改用第二种工具做桩，Java/Maven 用例随主仓库删除一起去掉） | |
| `…/tests/mybatis.rs@1daaeb525` | `dj/tests/mybatis.rs` | 已移植并有测试 | 10 个测试 |
| `…/tests/spring.rs@1daaeb525` | `dj/tests/spring.rs` | 已移植并有测试 | 8 个测试 |
| `…/tests/` 里的辅助函数 | `dj/tests/support.rs` | 已移植并有测试 | 2 个测试 |
| `…/tests/{detectors,build_systems,community_build_plugins,toolchain_resolution,project,project_generators}.rs@1daaeb525`、`lsp/tests.rs@1daaeb525`、`tests/fixtures/community_build_plugins/{maven,gradle}` | 无 | 不移植（通用或 Maven/Gradle 测试） | |
| `shared/fixtures/debug/java-test-launch-v1.json@1daaeb525` | `fixtures/debug/java-test-launch-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/java/run-markers-v1.json@1daaeb525` | `fixtures/java/run-markers-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/java/basic.json@1daaeb525` | `fixtures/java/basic.json` | 已移植无测试 | 契约样例，暂无 Rust 测试读取 |
| `shared/fixtures/lsp/java-completion-v1.json@1daaeb525` | `fixtures/lsp/java-completion-v1.json` | 已移植并有测试 | `jdt.rs` 读取 |
| `shared/fixtures/lsp/java-launch-preparation-v1.json@1daaeb525` | `fixtures/lsp/java-launch-preparation-v1.json` | 已移植并有测试 | `java_launch.rs` 读取 |
| `shared/fixtures/lsp/java-project-runtimes-v1.json@1daaeb525` | `fixtures/lsp/java-project-runtimes-v1.json` | 已移植并有测试 | `jdt.rs` 读取 |
| `shared/fixtures/lsp/java-workspace-policy-v1.json@1daaeb525` | `fixtures/lsp/java-workspace-policy-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/lsp/jdt-cache-retention-v1.json@1daaeb525` | `fixtures/lsp/jdt-cache-retention-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/lsp/jdt-readiness-v1.json@1daaeb525` | `fixtures/lsp/jdt-readiness-v1.json` | 已移植并有测试 | `jdt_progress.rs` 读取 |
| `shared/fixtures/lsp/jdt-workspace-fingerprint-v1.json@1daaeb525` | `fixtures/lsp/jdt-workspace-fingerprint-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/lsp/jdt-workspace-key-v1.json@1daaeb525` | `fixtures/lsp/jdt-workspace-key-v1.json` | 已移植并有测试 | `tests/java_commands.rs` 读取 |
| `shared/fixtures/lsp/project-preparation-v1.json@1daaeb525` | `fixtures/lsp/project-preparation-v1.json` | 已移植并有测试 | `project_preparation.rs` 读取；已去掉 Maven profile 阶段 |
| `shared/fixtures/lsp/java-build-report-v1.json`、`java-navigation-v1.json`、`jdt-direct-launch-v1.json@1daaeb525` | `fixtures/lsp/` 同名文件 | 已移植无测试 | 契约样例，暂无 Rust 测试读取 |
| `shared/fixtures/mybatis/basic.json`、`spring/basic.json@1daaeb525` | `fixtures/mybatis/basic.json`、`fixtures/spring/basic.json` | 已移植无测试 | 契约样例，暂无 Rust 测试读取 |
| `shared/fixtures/run-configuration/automatic-java-selection.json@1daaeb525` | `fixtures/run-configuration/automatic-java-selection.json` | 已移植并有测试 | `tests/java_selection.rs` 读取 |
| `shared/fixtures/lsp/maven-profile-events-v1.json@1daaeb525` | 无 | 不移植（Maven 插件） | |
| `shared/fixtures/execution/{maven,gradle,maven-java-main-source-sets-v1,maven-runtime-versions}.json@1daaeb525` | 无 | 不移植（Maven/Gradle 插件） | |
| `shared/fixtures/execution/standalone-java-compile-run-v1.json@1daaeb525` | 无 | 等待宿主 API（API-29） | 无项目文件的单文件编译运行属于运行准备，等通用运行配置接口 |
| `shared/fixtures/run-configuration/{hybrid-spring-vue,maven-module-ownership,provider-prepared-java}.json@1daaeb525` | 无 | 不移植（通用运行配置与 Maven 归属，留在 Core） | |
| `shared/fixtures/projects/lithe-spring-boot-git-graph/**@1daaeb525` | 无 | 不移植（Git 图示例工程，不是 Java 插件的测试输入） | |
| `shared/contracts/maven-launch-context-v1.schema.json`、`maven-portable-configuration-v1.schema.json@1daaeb525` | 无 | 不移植（Maven 插件） | |

### 3.3 macOS Swift 源码

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `macos/Sources/Lithe/Platform/MacOS/RunConfiguration/MacJavaLaunchArgumentPreparer.swift@1daaeb525`、`macos/Sources/LitheExecutionModule/Services/JavaLaunchArguments.swift@1daaeb525` | `dj/execution/argfile.rs` | 已移植并有测试 | 租约和命令规划在 Rust；文件 I/O 留给后端 |
| `…/Platform/MacOS/Plugins/HostIntegrations/Jdt/MacJdtLaunchResources.swift@1daaeb525` | `dj/jdt_integration/launch_resources.rs` | 已移植并有测试 | 3 个测试 |
| `…/Platform/MacOS/Plugins/HostIntegrations/Jdt/MacJdtHostIntegration.swift@1daaeb525` | `dj/jdt_integration/runtime.rs`、`dj/jdt_integration/mod.rs` | 已移植并有测试 | runtime 4 个测试；启动入口本身等宿主，见下一行 |
| 同上，启动 JDT LS 进程本身 | `dj/jdt_integration/mod.rs`（TODO 说明） | 等待宿主 API（API-25） | 由插件声明的语言服务器启动取代 |
| `…/Platform/MacOS/Plugins/HostIntegrations/Jdt/MacJdtWorkspaceState.swift@1daaeb525` | `dj/jdt_integration/workspace_state.rs` | 已移植并有测试 | 6 个测试。缓存目录来自宿主，见第 6 节 API-20† |
| `…/Services/Java/ProjectRuntimeService.swift@1daaeb525` 里的 JDT 运行时规则（版本解析、Java 21 下限、JDK home 推导） | `dj/jdt_integration/runtime.rs` | 已移植并有测试 | 探测 `java -version` 的 I/O 不移植，见第 6 节 API-23† |
| 同上，其余（JDK 发现、Maven 选择、运行时设置） | 无 | 不移植（宿主通用的运行时发现与设置，Maven 部分归 Maven 插件） | |
| `…/Services/Java/JavaCodeVisionService.swift@1daaeb525` | `dj/languages/code_vision_hints.rs` | 已移植并有测试 | 2 个测试。blame 作者来自宿主 Git，见第 6 节 API-42† |
| `…/Services/Debug/JavaTestDebugLaunchService.swift@1daaeb525` | `dj/testing/debug_launch.rs` | 已移植并有测试 | 4 个测试 |
| `…/Application/Features/JavaTestWorkflowState.swift@1daaeb525`、`JavaTestDebugWorkflowCoordinator.swift@1daaeb525` | `dj/testing/workflow.rs` | 已移植并有测试 | 11 个测试 |
| `…/Core/Debug/JavaTestResultServer.swift@1daaeb525`、`…/Platform/MacOS/Debug/MacJavaTestResultServer.swift@1daaeb525` | `dj/testing/result_channel.rs` | 已移植并有测试 | 7 个测试；按无 I/O 状态机重写。127.0.0.1 监听本身等宿主，见第 6 节 API-29 |
| `…/Services/Debug` 与 `LanguageToolingSessionManager.swift@1daaeb525` 里的 Java 测试分支（`discoverJavaTestItems`、`javaBuildTestSelectors`、`resolveJavaTestDebugLaunchTarget`、`javaTestLaunchArguments`） | `dj/testing/jdt_test_items.rs` | 已移植并有测试 | 12 个测试 |
| `…/LitheLanguageIntelligenceModule/Services/LanguageToolingSessionManager.swift@1daaeb525` 里的 Java 会话调度（导入等待、重启、`updateJavaMavenConfiguration`） | 无 | 等待宿主 API（API-25） | 会话由宿主通用管理；Maven 导入重启走 API-35，见第 5 节 |
| `…/Application/Composition/JavaLanguageServerPreparationCoordinator.swift@1daaeb525`、`Models/AppModel/AppModel+JavaBuildPreparation.swift@1daaeb525` | 无；状态规则在 `dj/lsp/languages/project_preparation.rs` | 等待宿主 API（API-25） | 宿主展示插件上报的准备状态 |
| `…/Models/AppModel/AppModel+JavaRunMarkers.swift@1daaeb525`、`AppModel+JavaEntrypoints.swift@1daaeb525` | 无；投影规则在 `java_run_markers.rs`、`java_entrypoints.rs` | 等待宿主 API（API-27） | 编辑器运行/调试标记由插件提供 |
| `…/Models/AppModel/AppModel+JavaNavigation.swift@1daaeb525`、`Application/Features/JavaFeatureModel.swift@1daaeb525` | 无；规则在 `java_navigation_syntax.rs`、`jdt_navigation.rs` | 等待宿主 API（API-27） | 实现/父声明导航标记由插件提供，跳转请求经 API-25 |
| `…/Models/AppModel/AppModel+JavaLaunchDecision.swift@1daaeb525`、`AppModel+JavaLaunchPreparation.swift@1daaeb525`、`Views/Run/JavaLaunchDecisionDialog.swift@1daaeb525`、`Views/Run/JavaRunConfigurationEditorView.swift@1daaeb525` | 无；规则在 `java_launch.rs`、`java_selection.rs` | 等待宿主 API（API-29） | 运行配置编辑与启动决策由宿主通用界面承载，Java 字段由插件声明 |
| `…/Models/AppModel/AppModel+JavaIndex.swift@1daaeb525`、`Application/Features/SpringFeatureModel.swift@1daaeb525`、`MybatisFeatureModel.swift@1daaeb525`、`Core/Rust/RustJavaMavenOperations.swift@1daaeb525` | 无；索引规则在 `spring.rs`、`mybatis.rs` | 等待宿主 API（API-25） | 补全、悬浮、诊断、导航由插件作为语言提供者给出 |
| `…/Views/Run/SpringEndpointsView.swift@1daaeb525` | 无 | 等待宿主 API（API-27） | Endpoints 面板需要插件原生界面的承载方式；本文不新增编号，落点待协调者确认 |
| `…/Theme/JavaFileIconResolver.swift@1daaeb525` | 图标在 `io.github.lithe-idea.java-support/icons/` | 等待宿主 API（API-27） | 文件图标由插件贡献 |
| `…/Application/Features/JavaPluginTransition.swift@1daaeb525`、`AppModel+JavaPlugin.swift@1daaeb525` | 无 | 不移植（宿主"推荐安装 Java Support"的引导，应由商店索引取代，不是插件逻辑） | |
| `…/Views/Language/JavaProblemsView.swift@1daaeb525`、`JavaReferencesView.swift@1daaeb525` | 无 | 不移植（宿主通用的问题和引用视图，没有 Java 领域规则，只需改名） | |
| `…/Models/Java/{JavaDiagnosticModels,JavaNavigationModels,MybatisModels,SpringModels,ActiveTestTarget}.swift@1daaeb525` | 无；JSON 形状在 `dj/protocol/contracts.rs` | 不移植（宿主界面模型） | |
| `…/Models/Java/{MavenModels,MavenResolutionProblems}.swift@1daaeb525` | 无 | 不移植（Maven 插件） | |
| `…/Models/Java/JavaRunModels.swift@1daaeb525`、`Services/Java/{JavaRunService,RunExecutableResolver,RunLaunchEnvironment,RunToolchainMetadataResolver}.swift@1daaeb525` | 无 | 不移植（宿主通用的运行与工具解析，只是放在 Java 目录里；`RunToolchainMetadataResolver` 的探测就不含 Java 分支） | |

### 3.4 macOS Swift 测试

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `macos/Tests/LitheLanguageIntelligenceModuleTests/JavaRunMarkerTests.swift@1daaeb525`（4 个） | `java_run_markers.rs` 的测试 | 已移植并有测试 | 光标选择最内层测试、回退到 main、测试方法的 Maven 选择器与 Java 测试项调试、载荷解码 |
| `macos/Tests/LitheTests/MacJdtWorkspaceStateTests.swift@1daaeb525`（4 个） | `dj/jdt_integration/workspace_state.rs` 的测试 | 已移植并有测试 | 指纹、清索引、缓存清理 |
| `…/MacJdtHostIntegrationStateTests.swift@1daaeb525`（4 个） | `dj/jdt_integration/runtime.rs`（`state_is_removed_only_when_no_installed_plugin_declares_the_integration`）与 `workspace_state.rs`（`removing_all_state_keeps_other_servers_caches`） | 已移植并有测试 | 卸载带走状态、扫描失败不删状态的规则；与宿主磁盘扫描有关的断言留给宿主测试 |
| `…/JavaTestDebugLaunchServiceTests.swift@1daaeb525`（9 个） | `dj/testing/debug_launch.rs`、`dj/testing/workflow.rs` 的测试 | 已移植并有测试 | 取消、替换、结果服务器释放；"暂停时显示调试器并激活应用"属宿主界面，不在其中 |
| `…/MacJavaTestResultServerTests.swift@1daaeb525`（2 个） | `dj/testing/result_channel.rs` 的测试 | 已移植并有测试 | 以状态机事件驱动，不开真实监听 |
| `…/JavaLanguageServerRuntimeTests.swift@1daaeb525` 里的准备阶段 fixture、Java 版本解析、Java 21 下限、JDK `release` 文件 | `project_preparation.rs`、`runtime.rs`、`argfile.rs` 的测试 | 已移植并有测试 | 共 5 个场景 |
| 同上，插件 JDK 选择（3 个）、准备任务取消与替换、导入等待、virtual document（约 10 个） | 无 | 等待宿主 API（API-25） | 属宿主会话管理的编排测试 |
| 同上，Maven 导入重启（3 个）、Maven runtime fixture | 无 | 不移植（Maven 插件，重启走 API-35） | |
| `…/MybatisFeatureModelTests.swift@1daaeb525`（7 个）、`SpringFeatureModelTests.swift@1daaeb525`（4 个） | 无；规则测试在 `tests/mybatis.rs`、`tests/spring.rs` | 等待宿主 API（API-25） | 这些是宿主功能模型的调度与跳转测试，要等提供者接线后在宿主测试里重做 |
| `…/RealJavaDebugIntegrationTests.swift@1daaeb525`（Spring/JUnit/TestNG 断点，需要真实 JDK） | 无 | 等待宿主 API（API-30†） | 真实调试端到端，调试适配器接线后在本仓库做 e2e |
| `…/Support/NoProjectJavaOperations.swift@1daaeb525` | 无 | 不移植（宿主测试替身） | |
| `macos/Tests/LitheTests/{LegacyMavenConfigurationMigrationTests,MacLegacyMavenRunConfigurationMigrationTests,MavenRuntimeTests,WorkbenchMavenLayoutTests,AgentMavenAppearanceTests}.swift@1daaeb525` | 无 | 不移植（Maven 插件或宿主界面） | |

### 3.5 插件包、脚本、CI、资源

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `Plugins/mac/Official/JavaSupport/plugin.json@1daaeb525` | `io.github.lithe-idea.java-support/plugin.json` | 已移植并有测试 | 身份改为普通插件：id 从 `dev.lithe.plugin.java-support` 变为 `io.github.lithe-idea.java-support`，`vendor` 改为 `Lithe Community`（`signatureRequirement: none`），删除 `publisherPackage`、`bundledSignatureRequirement` 和 `requires.packageTrust`，见第 9 节；版本仍是 1.1.0。新增后端声明：顶层 `backend`、`pluginFunctions`，`requires.backend: 2`、`pluginFunctions: 1`、`languageServers: 3`，权限 `backend.launch`、`languageServer.request`、`workspace.metadata`，激活事件 `onWorkspaceOpen`，语言服务器的 `requestMethods` 与 `commands`（第 5.7 节）；`launch` 改为后端解析、删除 `hostIntegration`（第 5.8 节）。`repository_pins.rs` 断言社区身份和后端启动的形状 |
| `Plugins/mac/Official/JavaSupport/{icons,templates}/**@1daaeb525` | `io.github.lithe-idea.java-support/{icons,templates}/` | 已移植并有测试 | `backend/tests/repository_pins.rs` 检查清单里每个文件都存在。Maven 图标（`icons/maven*.svg`）、Maven 与 Gradle 构建文件模板（`templates/java/{maven,gradle-kotlin,gradle-groovy,standard-*}`）已移交 Maven、Gradle 插件，本仓库只保留 `templates/java/plain*` 与 Java、Spring Boot 图标，见第 7 节 |
| `Plugins/mac/Official/JavaSupport/assemble-package.sh@1daaeb525` | `scripts/assemble-package.sh` | 已移植无测试 | 新增后端二进制：按 `--arch` 用 `cargo build --release --locked -p lithe-java-backend --target <triple>` 编译，`universal` 用 `lipo -create` 合并，放到 `plugin.json` 的 `backend.launch.path`（`bin/lithe-java-backend`）。由 `scripts/verify-package.sh` 校验产物，包括后端存在、可执行、架构与 `--arch` 一致 |
| `scripts/macos/prepare/prepare-jdtls.sh@1daaeb525`、`prepare-jdk.sh@1daaeb525` | `scripts/prepare-jdtls.sh`、`scripts/prepare-jdk.sh` | 已移植无测试 | 来源地址和 SHA-256 固定在 `third_party/*/manifest.json`，`repository_pins.rs` 检查格式 |
| `third_party/jdtls/manifest.json@1daaeb525`、`third_party/jdk/manifest.json@1daaeb525` | `third_party/jdtls/manifest.json`、`third_party/jdk/manifest.json` | 已移植并有测试 | 每个地址是 HTTPS，每个校验和是小写 SHA-256 |
| `.github/workflows/ci-plugins.yml@1daaeb525`、`release-official-plugins.yml@1daaeb525` 里 Java 的部分 | `.github/workflows/ci.yml`、`release.yml` | 已移植无测试 | 发布流程没有签名步骤，不读任何密钥，见第 9 节 |
| `scripts/plugins/{build,verify}-official-plugins.sh@1daaeb525` 里 Java 的部分 | `scripts/build-package.sh`、`archive-package.sh`、`verify-package.sh` | 已移植无测试 | 主仓库脚本里构建并调用签名器（`LithePluginPackageSigner`）和官方校验器（`LitheOfficialPluginVerifier`）的部分不移植：普通插件包不带发布者签名，所以不需要它们，也不需要 Swift 工具链；`verify-package.sh` 改为要求 `vendor.signatureRequirement` 为 `none` 且包里没有 `lithe-plugin-signature.json`，见第 9 节 |
| `scripts/windows/prepare/prepare-jdtls.ps1@1daaeb525`、`prepare-jdk.ps1@1daaeb525` | 无 | 不移植（Windows 插件包不在本阶段；Windows 仍在用） | `third_party/jdk/manifest.json` 保留了 Windows 条目，供以后使用 |
| `scripts/resources/worktree-resources.json@1daaeb525` 里 Java 的条目 | 无 | 不移植（主仓库的工作树复用清单，随主仓库删除同步更新） | 见第 8 节 |
| `scripts/architecture/verify-java-semantic-ownership.mjs@1daaeb525` | 无 | 不移植（由主仓库新的"宿主不含语言专属内容"闸门取代） | |
| `Plugins/marketplace/official-plugins.json@1daaeb525` 的 Java 条目 | 无 | 不移植（普通插件由 Lithe Store 收录，不进主仓库的索引；主仓库内置的官方条目在阶段 E 退役） | 第 8.2 节、第 9 节 |

### 3.6 Windows 与共享前端、功能矩阵、文案

| 来源 | 新位置 | 状态 | 说明 |
| --- | --- | --- | --- |
| `windows/tauri/src-tauri/src/lsp/jdt_workspace.rs@1daaeb525`、`src-tauri/src/lsp.rs@1daaeb525`（`jdt_workspace_key`、`resolve_jdtls_launch_resources`）、`tauri.jdtls.conf.json@1daaeb525` | 无 | 不移植（Windows 壳；Windows 宿主还没有插件运行时） | 阶段 W1/W2 才能删，见第 8 节 |
| `windows/tauri/src-tauri/src/maven.rs@1daaeb525` 与 `src/features/maven/**` | 无 | 不移植（Maven 插件，Windows 同样要等 W1） | |
| `windows/tauri/src/features/{spring,mybatis}/**`、`features/editor/lsp/java-*.ts`、`features/editor/engines/monaco/java-*.ts`、`features/run/services/java-*.ts`、`features/file-explorer/**/java-*.ts`、`platform/java-*.ts`、`utils/java-name-validation.ts` | 无 | 不移植（Windows 界面壳，没有 Java 领域规则的副本之外的逻辑） | 同上 |
| `frontend/editor/src/` 里的 Java 标记、`lithe.javaRun.*`、`lithe.javaNavigation.*` 动作 | 无 | 等待宿主 API（API-27） | 改成通用编辑器动作，标题由插件按语言提供 |
| `shared/platform-feature-matrix/features/*java*`、`*spring*`、`*mybatis*` | 无 | 不移植（功能矩阵在主仓库，归属改为"由插件提供"） | 第 8 节 |
| `shared/i18n/messages.json@1daaeb525` 里 Java 相关词条 | 无 | 等待宿主 API（API-37†） | 插件文案由插件自己提供，需要宿主 l10n 通道；目前后端的错误消息是英文 Core 消息 |

## 4. 交给 Maven/Gradle 插件的内容

### 4.1 从 Java 移植结果里删除的内容

以下内容不在本仓库，也不应该再回到 Java 插件：

- `rust/lithe-core/src/lsp/languages/jdt_maven_settings.rs`：为 JDT 生成 Maven settings。
- `rust/lithe-core/src/lsp/languages/jdt.rs` 里的 Maven 与 Gradle 部分：
  - Maven：`userSettings`、`globalSettings`、`sourcePaths`、`profiles`、`project_uris`、`localRepository`。
  - Gradle：`enabled`、`wrapper.enabled`、`offline.enabled`、`user.home`。
- `jdt_build.rs` 里的 Maven profiles 阻塞项，`project_preparation.rs` 里的 Maven profile 阶段。
- 命令 `lsp.retryMavenProfiles`、`lsp.updateMavenConfiguration`。
- `rust/lithe-core/src/build/language_import.rs`，以及主仓库 `jdt-maven-settings` 工作树资源（`<platform-cache>/jdtls/<workspace-state>/.lithe/maven/`）。
- `lib.rs` 公开的 `settings_with_local_repository`（把本地仓库路径写进派生 settings.xml）。

### 4.2 交付来源

交给 Maven/Gradle 插件的代码以主仓库的原文件为准，固定在 preview 提交 `dbe8441e2`：

- [`lsp/languages/jdt_maven_settings.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/lsp/languages/jdt_maven_settings.rs)：Maven settings 的生成与合并，含测试。
- [`project/maven.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/project/maven.rs)：`MavenJdtConfiguration`（`profiles`、`settings_path`、`global_settings_path`、
  `local_repository_path`、`project_paths`、`source_paths`）和 `settings_with_local_repository`。
- 移植时对照的改造前版本：同一提交下的 [`lsp/languages/jdt.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/lsp/languages/jdt.rs)、
  [`jdt_build.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/lsp/languages/jdt_build.rs)、[`project_preparation.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/lsp/languages/project_preparation.rs)、
  [`mod.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/lsp/languages/mod.rs) 和 [`build/language_import.rs`](https://github.com/1lck/Lithe-IDEA/blob/dbe8441e27486fef26f0e915e8c6a862c307e180/rust/lithe-core/src/build/language_import.rs)。

### 4.3 键归属表

| JDT LS 设置键 | 谁计算 | 怎样到达 JDT LS |
| --- | --- | --- |
| `java.configuration.maven.userSettings`、`java.configuration.maven.globalSettings` | Maven 插件（settings 文件，localRepository 写进它自己状态目录里的派生 settings.xml） | `languageImport.settings` 的片段，Java 校验前缀后合并 |
| `java.import.maven.*` | Maven 插件 | 同上 |
| `java.project.sourcePaths` | Maven 插件（从 POM 读出的源码目录） | 同上 |
| `java.import.gradle.enabled`、`wrapper.enabled`、`offline.enabled`、`user.home` | Gradle 插件（offline、gradleUserHome） | 同上 |
| Maven profiles（`java.project.updateSettings` 命令，参数 `org.eclipse.m2e.core.selectedProfiles`） | Maven 插件 | settings 片段无法表达；经 `languageImport.settingsChanged` 的 `projects` 条目交给 Java 后端，由后端发命令，见第 5.4 节 |
| `java.maven.downloadSources: false` | Java 插件自有 | 随 Java 自己的 `settings.java` 推送，不属于 Maven 逻辑 |
| 补全、导入排除、索引根、`java.configuration.runtimes` 等 | Java 插件 | 随 Java 自己的 `settings.java` 推送 |

## 5. 跨插件函数设计（API-35）

状态：**已实现**（Java 一侧）。接口常量、答案校验与合并在 `dj/lsp/languages/jdt_contributions.rs`（纯逻辑，不依赖 SDK），
发现并调用提供方在 `backend/src/plugin_functions.rs`，按工作区维护贡献、推送给 JDT LS、接收 `languageImport.settingsChanged` 的在
`backend/src/settings_session.rs`。宿主中转已由 Core 的 [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 实现并合入，后端按已合入的
方法形状（插件函数协议 4，Rust SDK 0.3.0）调用它，清单已声明（第 5.7 节）。仍缺的只有 `java.configuration.runtimes` 的推送，
它依赖 API-23，见第 6 节。

### 5.1 接口函数 ID

两个函数，版本都是 1。`languageImport.settings` 的参数和返回值各是一个对象：

- `languageImport.settings`：Java 定义的接口名，由 Maven 和 Gradle 插件各自导出。调用方 Java 只写这个 ID，不写对方插件 ID。
  - 参数：`{ "workspaceRoot": "<工作区根>" }`（`settings_request`）。
  - 返回：`{ "settings": { "java.import.maven.enabled": true } }`。`settings` 可以嵌套，也可以用点分键；未知的同级字段被忽略，
    以后加字段不破坏这一版。
- `languageImport.settingsChanged`：Java 导出的变化通知函数（`context: "workspace"`）。Maven/Gradle 在设置变化时调用，
  参数是 `{ "settings": {…}, "projects"?: [{ "projectUri", "selectedProfiles": [...] }] }`，`settings` 与上面的返回值同形，
  携带完整的新设置（不是差量）。Java 后端按 Core 写在调用里的调用方插件 ID（`caller`，不能由参数伪造）**整份替换**该调用方的槽位，
  不影响别的提供方；随后用 `workspace/didChangeConfiguration` 把合并后的设置推给 JDT LS（`languageServer/notify`）。
  `projects` 见第 5.4 节。

### 5.2 时序

1. 宿主通知 `lithe/host/languageServer/didChangeState`（`serverID` 为 `java`）说语言服务器进入 `running`，并给出该服务器进程的代号。
   后端记下代号；远程工作区（取不到本地根路径）不收集。`stopped`、`crashed` 只标记不再运行，保留已有贡献，下次 `running` 重新收集后覆盖。
2. 向宿主发现已启用且导出 `languageImport.settings` 的插件（`lithe/host/functions/providers`）。结果按插件 ID 排序、去重，
   只保留导出版本等于契约版本（1）的提供方，最多取 16 个（`MAX_PROVIDERS`），避免插件很多时拖慢收集。
3. 逐个调用（`lithe/host/functions/invoke`），每个提供方最多等 10 秒（`PROVIDER_TIMEOUT`，经 `timeoutMilliseconds` 传给宿主）。
   超时只取消那一次请求，不影响其他提供方。
4. 每个答案交给 `accept_answer`：落在 `CONTRIBUTED_SETTINGS_KEYS` 里的叶子键被采用，其余记入 `rejected`，永远不会到达 JDT LS；
   日志里只上报类别 `relay.keys_rejected` 和计数，不写键名。
5. 合并按插件 ID 排序，叶子键相同时后者覆盖前者。收集期间代号变了（服务器重启过）则丢弃结果。
6. **只有至少一个提供方参与（回答或失败）时才推送**：用 `languageServer/notify` 发 `workspace/didChangeConfiguration`，
   `settings` 整份替换会话里的插件配置。没有任何提供方时什么都不推送——过渡期主仓库的 jdt 宿主集成仍在推送它自己的设置，
   插件这边的整份替换会把它覆盖掉；在 Maven/Gradle 插件导出 `languageImport.settings` 之前必须零行为变化。
7. 推送里 `java.configuration.runtimes` 为空：插件后端目前没有列出 JDK 安装的宿主方法（API-23 `toolchains.list`），
   等它提供后把宿主的候选列表填进来；JDT LS 不会因为缺这个键而删除已登记的 VM。索引根也不在插件推送里，因为 Core 会在插件配置上叠加宿主的索引策略。

### 5.3 超时、失败与状态

- 超时或失败都不阻塞打开窗口：按已收到的设置推送。一个提供方失败不影响其他提供方。
- 调用函数自己被取消是唯一的错误（`Cancelled`）；其余失败都记在 `Contributions` 里。
- `ContributedSettingsStatus` 表示状态：`Settled`（全部回答，或没有关联插件）、`Pending`（仍在收集或应用，由调用方在收集期间持有）、
  `Failed`（有提供方失败、超时、答案不可用，或宿主根本无法列出提供方）。`Pending` 时 `java_build` 的阻塞项是 `ContributedSettings`
  （`waitingForContributedSettings`），和原来等待 Maven profiles 的位置相同。
- 状态行里的失败原因只带稳定的错误码（例如"The call failed (plugin.host.timeout)."），不带宿主返回的消息，因为消息可能带路径；
  详细消息（`details`）不离开后端，失败只以 `pluginErrorCode` 为类别经 `litheLogging` 1 上报；
  中转降级另有 `relay.*` 类别。级别与类别见 README 的“日志”一节。
- 不在 `languageImport.settings` 的调用里回调 Maven，避免调用链环路。

### 5.4 Maven profiles：经 `projects` 交付

JDT LS 1.61.0 没有 Maven profiles 的设置键。Core 原来只能用 `java.project.updateSettings` 命令加
`org.eclipse.m2e.core.selectedProfiles` 逐工程应用。settings 片段表达不了这件事，所以：

- profiles 仍由 Maven 插件计算。
- `languageImport.settingsChanged` 除了 `settings`，还接受 `projects`：逐工程的条目 `{projectUri, selectedProfiles}`。
  Java 后端按 `projectUri` 去重（保留第一次出现），把 `selectedProfiles` 用 `,` 连接，经 `languageServer/request` 发
  `workspace/executeCommand`，命令为 `java.project.updateSettings`，参数是 `[projectUri, {"org.eclipse.m2e.core.selectedProfiles": "<连接结果>"}]`。
  `plugin.json` 的语言服务器因此声明 `requestMethods: ["workspace/executeCommand"]` 和 `commands: ["java.project.updateSettings"]`
  （权限 `languageServer.request`）。
- 语言服务器尚未运行时，`projects` 先暂存，等进入 `running` 并推送一次设置之后再发。
- 条目形状与 Maven 插件的对接仍要在 lithe-plugin-maven 一侧确认；`project_update_notification(project_uris)` 保留着通用部分
  （`java/projectConfigurationsUpdate`），在 `dj/lsp/languages/jdt.rs`。

### 5.5 工具链要求

Maven 和 Gradle 对 JDK 版本的要求（例如 Maven 工程的 `maven.compiler.release`）不由 Java 读取。
`dj/execution/toolchain_requirements.rs` 只保留版本比较函数，要求本身以后由构建插件通过 API-35 的函数提供。

### 5.6 测试运行

Java 的编辑器标记（API-27）被点击后，Java 后端只产生测试选择器（`build_test_selectors`）。运行由拥有该工程的构建插件完成：
Java 后端向宿主查询导出测试运行函数 ID 的已启用插件，把选择器发过去，不写死 Maven 或 Gradle。结果显示在宿主原生测试面板的落点，
等 API-27 的编辑器动作 API 就位后确定。

### 5.7 中转协议形状与清单声明（已实现）

`backend/src/plugin_functions.rs` 是后端里唯一知道中转协议的地方，按 Core 已合入的中转
[#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 的形状调用 Rust SDK 0.3.0 的 `Host::functions()`（以主仓库
`shared/contracts/plugins/v1/README.md` 的 “Discovery and invocation” 一节为准）：

| 宿主方法 | 形状 |
| --- | --- |
| `lithe/host/functions/providers` | 参数 `{ functionID }`，结果 `{ providers: [{ pluginID, version }] }`，按插件 ID 排序、不含调用方自己；`version` 是对方导出的函数版本 |
| `lithe/host/functions/invoke` | 参数 `{ pluginID, functionID, context: { contextID, generation }, params?, timeoutMilliseconds?, parentOperationID? }`，结果 `{ value }`；未定义的字段是 `-32018`（`plugin.host.unsupportedOption`） |

两个方法都只对协商到插件函数协议 4 的后端开放，协议 3 及以下得到 `-32601`；Rust SDK 0.3.0 协商协议 4，`context` 和嵌套调用的 `parentOperationID`
由 SDK 填。收集逻辑只依赖 `FunctionRelay` 这个 trait，测试用桩实现，不连宿主。SDK 按已发布的标签 `sdk-rust-v0.3.0` 固定（提交 `db17c719`，
见 `sdk.lock.json`）。

Java 的清单现在这样声明（以已合入的契约 README 为准）：

- 顶层 `backend.launch`：`{ "kind": "packaged", "path": "bin/lithe-java-backend" }`。后端不读命令行参数，所以没有 `arguments`
  （不再用 `--state ${stateDir}`；需要工作区状态目录时读宿主工作区上下文里的 `stateDirectory`）。打包脚本把按架构编译好的二进制放到这个路径，`verify-package.sh` 检查它。
- `pluginFunctions.calls` 为 `languageImport.settings`（版本 1）；`pluginFunctions.exports` 为 `languageImport.settingsChanged`（版本 1，`context: "workspace"`），
  两个数组按 `id` 排序。
- `requires` 写 `backend: 2`、`pluginFunctions: 1`、`languageServers: 3` 和 `serverProcess: 1`，`pluginAPIVersion` 为 4；
  后端运行时协商到插件函数协议 4 才能调用中转方法。
- 权限：`backend.launch`、`languageServer.request`（语言服务器列了 `requestMethods`）、`workspace.metadata`（读取工作区根路径需要 `backend` 2）。
- `languageServers[0]` 声明 `requestMethods: ["java/classFileContents", "workspace/executeCommand"]` 和 `commands: ["java.project.updateSettings"]`；
  `launch` 是 `{ "kind": "backend", "function": "java.resolveLaunch", "timeoutMilliseconds": 60000 }`（`requires.serverProcess` 3），
  没有 `hostIntegration`、`arguments`、`runtimeDependencies` 和 `initializationOptions`，它们的值都由后端在第 5.8 节的回答里给出。

### 5.8 语言服务器启动（serverProcess 3）

`backend/src/launch.rs` 注册 `java.resolveLaunch`。宿主按契约的 `resolveParams`（`serverID`、`platform.arch`、`stateDirectory`、
`workspaceStateDirectory`、`logDirectory`、`temporaryDirectory`、`settings`）加工作区上下文调用它，后端：

- 以后端二进制所在的 `bin/` 的上级为包根，按固定布局定位文件：`language-server/plugins/org.eclipse.equinox.launcher_*.jar`（恰好一个）、
  `language-server/configuration/<arch>`、`language-server/lombok/lombok.jar`、`language-server/java-debug` 与 `language-server/java-test/extensions`
  里的扩展包、`runtime/<arch>/bin/java`。布局常量是 `pub`，`backend/tests/repository_pins.rs` 用它们约束 `scripts/assemble-package.sh`
  和 `scripts/verify-package.sh`，所以 `plugin.json` 不再写这些路径；
- 把 `heapMaxMB`、`logLevel` 两个设置和 `logDirectory/jdt.log` 拼进 JVM 参数（原 `runtimeArguments` 的 `${setting:…}` 与 `${logDir}` 展开），
  把每个工作区的 Equinox 配置复制到 `workspaceStateDirectory` 下再以 `-configuration` 传入，`-data` 指向其中的 `data/`；
- 回答 `executable: { kind: "packaged", path }`、`arguments`、`initializationOptions`（原 `hostIntegration` 的 bundles、设置与
  `java_configuration`）、`workspaceConfiguration`、`writablePaths`，以及一个 `language/status` 的就绪门（`service-ready`）。

错误码 `launch.invalid_params`、`launch.package_unavailable`、`launch.resource_missing`、`launch.configuration_failed` 与设置中转的错误码不重名。
- 激活事件加 `onWorkspaceOpen`：本插件要主动调用 `languageImport.settings` 并接收语言服务器的状态通知，只导出函数不足以启动后端。
  后端是全机一个进程，所有工作区共享。

**变化通知函数的调用方来源。** 槽位按提供方分：一次变化调用只替换发送者自己的那一份。可信的发送者只能来自宿主，
调用方插件 ID 由 Core 放在插件函数协议 4 的 `caller` 字段里，SDK 0.3.0 经 `Context::caller()` 提供；后端不读调用参数里的“我是谁”，
否则任何插件都能覆盖别人的设置。缺 `caller` 的调用（不是 Core 中转来的）被拒绝。

**信任边界。** 宿主只中转，不传递信任：Maven/Gradle 插件用自己的权限执行，Java 不因此获得权限；Java 只接受 `CONTRIBUTED_SETTINGS_KEYS`
里的键，所以一个提供方既不能改 `java.home`，也不能改别的插件的槽位。

### 5.9 运行、调试与测试（API-27 / API-29，已实现）

`backend/src/run.rs` 用三个宿主能力接管了主仓库 Core 与 macOS 原来写死的 Java 运行流程：

- **行内动作**（`editor/registerActions`，权限 `editor.actions` + `documents.read`）：后端在 `initialize` 之后与每次
  `didChangeContexts` 时列出工作区上下文，为每个上下文注册 `.java` 的收集函数 `java.collectActions`。宿主对每个打开的 Java 文档
  调用它，后端向 JDT LS 请求 `vscode.java.resolveMainMethod` 与 `vscode.java.test.findTestTypesAndMethods`，经领域规则
  `java_run_markers` 得到标记，每个标记一条 `Run '<label>'` 动作（图标 `run`/`test`）并带 `Debug` 备选项。语言服务器没起来、
  忙或超时时回答空列表，宿主在下次编辑时再收集。
- **启动进程**（`run/start`，权限 `run.start`；调试另需 `debug.start`）：点击调用 `java.runAction`。main 类先经
  `vscode.java.buildWorkspace`（结果 1 或 2 继续）再 `vscode.java.resolveClasspath` 得到 `[modulePaths, classPaths]`；
  测试经 `plan_debug_launch` → `vscode.java.test.junit.argument` → `debug_launch_target` → `java_test_launch` 得到运行器参数。
  进程用清单 `requiresTools: ["java"]` 引用的工具定义（`executable.kind: "tool"`），由用户在 JDK 设置里选择的 JDK 执行；
  调试时在最前面加 `-agentlib:jdwp=…,address=localhost:<port>` 与 `-Dspring.devtools.restart.enabled=false`，并在 `debug`
  字段给出 `{configurationType: "java", target: {kind: "connection", port}}`，宿主用清单的 `java` 调试适配器接上。
  `rerun` 指向 `java.rerun`，数据就是动作参数。
- **测试结果**：后端对每次测试启动在 127.0.0.1 开一个端口交给 Eclipse JUnit 运行器（`-port`），监听线程按 `%TESTS`/`%FAILED`/
  `%ERROR` 计数，运行器断开后用 `window/showInformation`（有失败用 `showWarning`）显示 `<名称>: N passed, N failed…`；
  宿主发 `run/didExit` 后停止。摘要文案是英文，等插件后端能用宿主 i18n（API-37）。
- **输出链接**（`outputLinkMatchers` 1）：清单的 `stack-frame` 匹配器捕获 `at pkg.Class.method(Class.java:12)`，
  点击调 `java.resolveOutputLink`，后端用 `workspace/findFiles`（权限 `workspace.search`）按文件名查找，
  优先包目录匹配的文件，回答 `{uri, range}`。
- **调试适配器声明**（`debugAdapters` 3）：热替换 `redefineClasses`/`hotcodereplace`/`changedClasses`/`errorMessage`
  与默认步进过滤 `$JDK`、`$Libraries` 写进清单的 `features`/`defaults`，主仓库不再按适配器 ID 特判。
- 清单改动：`requires.serverProcess: 4` + `requiresTools: ["java"]`、`requires.debugAdapters: 3`、
  `requires.outputLinkMatchers: 1`；`languageServers[].commands` 白名单加入上面五个 `vscode.java.*` 命令；权限加入
  `documents.read`、`editor.actions`、`run.start`、`debug.start`、`window.messages`、`workspace.search`。

## 6. `TODO(host-api)` 清单

每条都是代码里的占位注释，格式 `TODO(host-api: API-NN 说明)`；`†` 的含义见第 2.3 节。

主仓库里对应的宿主 PR（状态以 PR 页面为准）：

| 计划编号 | 宿主 PR |
| --- | --- |
| API-25 语言服务器会话、后端决定启动方式（含 API-12、20、22） | [#1745](https://github.com/1lck/Lithe-IDEA/pull/1745) |
| API-25、API-27 编辑器 Provider 与编辑器动作 | [#1730](https://github.com/1lck/Lithe-IDEA/pull/1730) |
| API-22、23、30 后端发起运行与调试、调试适配器行为、输出链接 | [#1746](https://github.com/1lck/Lithe-IDEA/pull/1746) |
| API-33 项目生成器通用工具字段 | [#1740](https://github.com/1lck/Lithe-IDEA/pull/1740) |
| API-35 跨插件函数中转（第一阶段） | [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) |
| 宿主语言特定内容闸门（已合入） | [#1727](https://github.com/1lck/Lithe-IDEA/pull/1727) |

### API-25 语言服务器

- `dj/commands.rs`：会话命令 `java.navigationMarkers`、`java.resolveNavigation`、`java.prepareLaunch` 现在回答 `NotSupported`。
  需要宿主把向声明的语言服务器发 `workspace/executeCommand` 与自定义请求、收通知的通道开放给插件后端。
- `dj/host/indexing.rs`：索引排除规则必须经宿主送到语言服务器的所有配置通道。
- `dj/testing/debug_launch.rs`：真实解析要向 JDT LS 发 Java 测试请求。
- `dj/jdt_integration/mod.rs`：宿主仍在自己运行这些启动与状态规则，要由语言服务器启动声明取代。
- `backend/src/lib.rs`（与 API-27、API-29 并列）：`plugin.json` 已声明 `backend`（`requires.backend: 2`，`requires.serverProcess: 1`，
  `backend.launch {kind: "packaged", path: "bin/lithe-java-backend"}`，第 5.7 节）。宿主能为编辑器提供者、运行配置、Java 测试流程调用插件函数之后，
  再把宿主每个调用点从 Core 切到这些函数。
- 语言服务器已改为 `launch.kind: "backend"` 并去掉 `hostIntegration`（第 5.8 节）；运行/调试/测试由后端接管（第 5.9 节）。

### API-27 编辑器动作与标记（运行/调试标记已完成）

- 运行与调试的行内标记由 `backend/src/run.rs` 通过编辑器动作（`editor/registerActions`）提供，见第 5.9 节。
- 导航标记（父类/实现）、CodeLens、`SpringEndpointsView`、`JavaFileIconResolver` 对应的 Swift 行仍等这个编号的其余部分。

### API-29 构建与测试（运行/调试/测试启动已完成）

- 运行、调试、测试的进程由后端经宿主 `run/start` 启动，调试会话由宿主按 `debug` 字段接到声明的 `java` 调试适配器，
  见第 5.9 节。
- `dj/testing/result_channel.rs`：后端自己在回环地址上监听 JUnit 运行器的结果端口（`backend/src/run.rs` 的结果监听器），
  不再需要宿主提供回环通道。
- `dj/testing/jdt_test_items.rs`（`build_test_selectors`）：选择器由构建插件运行，见第 5.6 节（与 API-35 并列）。
- 未做：`execution/launch_command.rs` 的 argfile 规划没有接到 `run/start`（命令行按 JDT LS 给的长度原样传递）；
  TestNG 运行器的结果协议不解析，只报“已结束”。

### API-33 项目模板

- 本仓库没有 `TODO(host-api: API-33)` 占位：两个项目生成器（`java`、`spring-boot`）是 `plugin.json` 里的
  声明式生成器，用的是已有的 `projectGenerators` 1。API-33 在计划里是"既有能力"。
- `maven-archetype` 生成器、Maven 与 Gradle 构建文件模板、Maven 图标已经移交 Maven、Gradle 插件（第 7 节）。

### API-35 跨插件函数（已完成）

Java 一侧的中转已经实现并在清单里声明（第 5 节、第 5.7 节），本节原来的占位已清除：

- 中转形状：后端按已合入的 [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742) 经 Rust SDK 0.3.0（插件函数协议 4）调用，已完成。
- `languageImport.settingsChanged`：已注册，调用方插件 ID 取自 `caller`，已完成。
- 变化后的 `workspace/didChangeConfiguration` 与 Maven profiles（`java.project.updateSettings`）：经 `languageServer/notify` 与
  `languageServer/request` 发出（#1745），已完成（第 5.2、5.4 节）。
- 还剩下的两项属于其他编号：`java.configuration.runtimes` 的推送等 API-23（`toolchains.list`），见下文；
  构建插件提供工具链要求和测试运行（第 5.5、5.6 节）仍是设计，没有实现。
  - `dj/execution/toolchain_requirements.rs`：构建插件通过函数提供工具链要求（第 5.5 节）。
  - `dj/testing/jdt_test_items.rs`：测试运行委托给构建插件（第 5.6 节）。

### 来自计划里其他编号的占位（`†`，不在指定的五个之内）

- API-20 存储：`dj/execution/argfile.rs`（后端没有插件自有的临时目录）、`dj/jdt_integration/workspace_state.rs`（缓存目录来自宿主）。
- API-22 日志与进度：日志部分已完成，后端接入 `litheLogging` 1，不再有占位（第 3.1 节“Java 日志事件”一行，级别表见 README 的“日志”一节）。
- API-23 工具链与进程：`dj/jdt_integration/runtime.rs`（探测 `java -version` 的进程执行）。
  `toolchains.list`（列出已安装 JDK）也属于它：插件后端没有这个宿主方法，所以设置中转暂不推送 `java.configuration.runtimes`
  （`backend/src/settings_session.rs` 里的 `TODO(host-api: API-23 toolchains.list)`），JDT LS 的 VM 登记保持现状。
  语言服务器已改为 `launch.kind: "backend"`（第 5.8 节），`java.configuration.runtimes` 的推送仍等这个方法。
- API-37 环境与 l10n：`dj/jdt_integration/runtime.rs`、`dj/jdt_integration/launch_resources.rs`、`backend/src/lib.rs`（用户可见文案要走宿主 i18n）。
- API-42 内置 Git 只读：`dj/host/git.rs`、`dj/languages/code_vision_hints.rs`（blame 作者）、`dj/lsp/languages/jdt_project_metadata.rs`（一条测试注释，Git 忽略查询）。
- API-30 调试适配器：第 3.1 与 3.4 节表里的 `debug/engine.rs` 特例与 `RealJavaDebugIntegrationTests`，代码里没有单独占位。

## 7. 判断记录

这些是移植中做的、值得让评审者知道的取舍：

- **工作区指纹**（`java_workspace.rs`）：只保留与 JDT 状态目录命名有关的部分，指纹里读取的是根构建文件的存在与直接模块目录，
  不解析 POM 内容。读取文件仍是"这个工作区结构变了没有"的判断，不是 Maven 逻辑。
- **`toolchain_requirements.rs` 保留**：JDK 选择要比较版本，这几个纯函数是复制过来的最小集合，不含 Maven/Gradle 要求。
- **`java.project.sourcePaths` 键**：Core 当时用这个键发 POM 里的源码目录，所以把它放进接受列表；键本身属于 JDT LS 的命名空间，值由 Maven 插件给。
- **`project_update_notification`**：是 JDT LS 的通用协议（`java/projectConfigurationsUpdate`），保留在 Java 一侧，Maven 插件改了设置后由 Java 发出。
- **`java.maven.downloadSources: false`**：留在 Java 自己的设置里。它是 JDT LS 的行为开关，不涉及 Maven 的配置值。
- **模板、图标与 `maven-archetype` 生成器（已移交）**：原来 `plugin.json` 里 `java` 生成器带 Maven/Gradle 构建文件模板，`maven-archetype`
  生成器运行 `mvn archetype:generate`，`icons/maven*.svg` 是 Maven 图标。它们是"新建项目"（API-33），不是"导入工程"，最初按已有行为一起移植，
  现在已按协调者的决定移交：
  - 移交 lithe-plugin-maven：`maven-archetype` 生成器、Maven 项目模板（`templates/java/maven`，即 `pom.xml`）、Maven 图标（`icons/maven.svg`、`maven_dark.svg`），
    以及 Maven 项目用的标准目录模板（`templates/java/standard-sample`、`standard-sample-main`、`standard-empty`）。
  - 移交 lithe-plugin-gradle：Gradle 模板（`templates/java/gradle-kotlin`、`gradle-groovy`）。原来 Maven 和 Gradle 项目共用同一组标准目录模板
    （`standard-*` 与 `src/main/java/<groupId>/Main.java` 的打开文件规则），Gradle 插件需要自带一份，不能指望引用 Maven 插件的文件。
  - 本仓库保留：`java` 生成器只剩“无构建系统”一种（字段 `jdk`、`addSampleCode`，模板 `templates/java/plain`、`plain-sample`、`plain-empty`），
    以及 `spring-boot` 生成器（Spring 不拆）。`buildSystem`、`gradleDsl`、`groupId`、`artifactId` 字段随之删除。
  - **在这两个插件接手模板之前不要发布本插件的新版本**，否则“新建项目”会少掉 Maven 和 Gradle 选项。
  - 依赖本仓库旧版 `java` 生成器（带 `buildSystem` 字段）的调用方需要改为由 Maven/Gradle 插件各自的生成器提供。
- **6 个没有 Rust 测试读取的 fixture**：`lsp/java-build-report-v1.json`、`lsp/java-navigation-v1.json`、`lsp/jdt-direct-launch-v1.json`、
  `java/basic.json`、`mybatis/basic.json`、`spring/basic.json`。它们是主仓库里的共享契约样例，复制过来为了让两边保持同一份样例；
  本仓库没有消费它们的类型测试。`repository_pins.rs` 只断言 fixture 数量不少于 18 且都是合法 JSON。
- **没有联网测试和 Linux 之外的平台**：所有测试都不联网；CI 的 Rust 任务在 Ubuntu 上跑，打包任务在 macOS 上跑。

## 8. 主仓库删除清单（分阶段）

不能一次删完：Windows 宿主还没有插件运行时，Windows 仍在用 Core 里的 Java 模块和 `third_party` 清单。顺序如下。

### 8.1 前置条件

- Core 的跨插件函数中转（API-35）合并，Maven/Gradle 插件导出 `languageImport.settings`。
- 宿主的语言服务器通道（API-25）、编辑器标记（API-27）、运行与测试（API-29）合并，Java 插件声明 `backend` 并通过 e2e。
- 本仓库发布一个支持 API 4 的预发布版本（普通插件，不带发布者签名），且 Maven、Gradle 插件已经接手项目模板（第 7 节）；转为正式发布要等
  主仓库不再内置 Java Support、Lithe 稳定版提供 `backend` 2、`languageServers` 3、`pluginFunctions` 1 和 `litheLogging` 1（见第 9 节）。
  原来的两条按官方身份设的宿主限制已由主仓库 [#1763](https://github.com/1lck/Lithe-IDEA/pull/1763) 取消（第 8.2 节）。

### 8.2 macOS 阶段（阶段 E）：Core 与 macOS 应用

本仓库的 README、发布工作流和 Release 说明里说的"主仓库阶段 E"，指的就是这一节的删除。

- Core：
  - 两条按官方身份设的宿主限制已由主仓库 [#1763](https://github.com/1lck/Lithe-IDEA/pull/1763) 取消，本插件可以作为普通插件被宿主接受：
    普通插件可以声明 `languageServers[].hostIntegration`（JDT）和短 `toolDefinitions` ID（例如 `java`）；
    多个插件争用同一个短工具 ID 时都不归属。本插件已不再声明 `hostIntegration`（第 5.8 节），Core 里的 JDT 宿主集成随主仓库的拆分 PR 删除。
  - `lsp/languages/{java_*,jdt*,project_preparation}.rs` 与 `mod.rs` 里的 re-export，`languages/{java,java_syntax,spring,mybatis}.rs`，
    `debug/java_test.rs`，`execution/{java_selection,launch_command,classpath_jar}.rs` 的 Java 部分，`protocol/contracts.rs` 里的 Java、Spring、MyBatis、Maven 模型。
  - `protocol/command.rs` 与 `runtime/dispatcher.rs` 里的 Java 命令；`engine.rs` 里的 Java 函数与 `StartServerRequest` 的 Java 字段；
    `engine_real_jdt*_tests.rs`。
  - `lsp.retryMavenProfiles`、`lsp.updateMavenConfiguration`、`build/language_import.rs`、`build/integrations/`、`jdt_maven_settings.rs`、
    `project/maven*.rs`（Windows 完成 W1 之前保留）、`detectors/{maven,gradle}.rs`（由 Maven/Gradle 插件取代之后）。
  - 通用代码里的 Java 特判：`KNOWN_RUNTIME_TOOLS*`、`HOST_INTEGRATION_*`、`ModelSource::HostIntegration`、`TestResultsSource::HostIntegration`、`SELECTED_RUNTIME_TOOLS`、
    `host_runtime_version_arguments` 的 java 分支。
  - 只服务 Java 的测试与 fixture（第 3.2 节标"已移植"的那些）。
  - `diagnostics` 里的 Java 日志事件：改 `shared/logging/events/*.json` 后重新生成。
- macOS：第 3.3 节标"已移植"或"等待宿主 API"的 Swift 文件与第 3.4 节的测试，`LitheIcons` 里的 Java 命名改成通用符号种类图标，
  `BuiltInModuleCatalog` 里的 Java 条目。
- 宿主资源与脚本：
  - `Plugins/mac/Official/JavaSupport` 迁出后删除；`scripts/macos/prepare/prepare-{jdtls,jdk}.sh`（Windows 的 `.ps1` 不动）。
  - 不再内置 Java Support：退役内置的官方条目 `dev.lithe.plugin.java-support`（`Plugins/marketplace/official-plugins.json` 里
    `releaseEnabled: false`、`bundledUntilPublishedVersion: "1.1.0"` 的那一条），并把用户迁移到新 ID `io.github.lithe-idea.java-support`。
    先例是 Go 和 PHP 在主仓库 [#1357](https://github.com/1lck/Lithe-IDEA/pull/1357) 改成社区插件：旧 ID 留在 `OfficialPluginCatalog`，
    用 `communityReplacementID` 指向替代者。本插件发布稳定版之后，由 Lithe Store 自动上架，不在主仓库的索引里加条目。
  - `scripts/resources/worktree-resources.json` 里 `jdtls`、`jdk`、`jdt-maven-settings`、`java-launch-temporaries`、Java 插件包路径的条目，
    以及复用脚本的校验路由、对应测试和 `docs/ci-builds.md` 的"独立工作树的本地编译"章节，要在同一个 PR 里同步更新（AGENTS.md 的规定）。
  - `scripts/architecture/verify-java-semantic-ownership.mjs` 由新的闸门取代；`ci-plugins.yml` 里的 `jdtls: "true"`，`release-official-plugins.yml` 里的路径。
- 文案与矩阵：删除宿主不再使用的 Java 词条，重新生成 i18n 并运行两个检查脚本；功能矩阵里的 Java 条目改为"由插件提供"。
- 契约：`shared/contracts/plugins/v1/` 里与 Java 专属相关的内容（`hostIntegration jdt`、`runtime:"java"` 字段等）按契约发布规则改，
  合并后在 lithe-plugin-docs 运行 `node scripts/contract.mjs sync --revision <合并后的 preview commit>`。

### 8.3 Windows 阶段：不属于第一批

- 保留到 W1：`windows/tauri/src-tauri/src/maven.rs`、`lsp.rs` 里的 `jdt_workspace_key`、`lsp/jdt_workspace.rs`、`src/features/{maven,spring,mybatis}`、
  各 `java-*.ts`、`lib.rs` 公开的 `jdt_workspace_key` 与 `settings_with_local_repository`、`third_party/{jdtls,jdk}/manifest.json`、
  `scripts/windows/prepare/prepare-{jdtls,jdk}.ps1`、`tauri.jdtls.conf.json`。
- 这些在 Windows 宿主能安装并运行本插件之前，不能删。

## 9. 发布与商店

### 9.1 身份与收录策略

- Lithe 不再区分官方插件和普通插件，所有插件都按普通插件处理，不带发布者签名。区别只在 [Lithe Store](https://github.com/Lithe-IDEA/Lithe-store)
  怎样收录：Lithe-IDEA 组织里 `lithe-plugin-*` 仓库的稳定版（非预发布）GitHub Release 由商店机器人自动跟踪并上架；
  组织之外的插件向 Lithe-store 提交 pull request，维护者审核后上架。
- 本插件的 id 是 `io.github.lithe-idea.java-support`，`vendor` 是
  `{ "id": "io.github.lithe-idea", "displayName": "Lithe Community", "signatureRequirement": "none" }`。
  它在主仓库里原来是 `dev.lithe.plugin.java-support`，`vendor` 为 `dev.lithe`，带 `publisherPackage` 和 `bundledSignatureRequirement: sameTeamAsHost`；
  这两项随官方身份一起删除。`requires.packageTrust` 也删除，因为它只服务 `vendor.bundledSignatureRequirement`。
  权限、设置和其余 `requires` 不变。版本仍是 1.1.0，新 ID 下还没有发布过任何版本。
- `backend/tests/repository_pins.rs` 断言这个身份（`vendor`、没有 `bundledSignatureRequirement` 和 `packageTrust`）；
  `scripts/verify-package.sh` 要求 `vendor.signatureRequirement` 为 `none`，并拒绝包里的 `lithe-plugin-signature.json`。
  两者共同保证插件包既不携带发布者签名，也不要求宿主去验证发布者签名。
- 主仓库里的签名器（`LithePluginPackageSigner`）和官方校验器（`LitheOfficialPluginVerifier`）没有移植，本仓库不构建也不使用它们，
  打包不再需要 Xcode 或 Swift。

### 9.2 发布流程

- 标签是 `v<version>`，必须等于 `plugin.json` 的 `version`（当前 1.1.0）。发布资产是 `Lithe-Java-Support-{version}-darwin-{arm64,x64}.zip`
  加 `.sha256`，由 `scripts/archive-package.sh` 用 `ditto --keepParent` 制作；本地开发、CI 和发布用的是同一个脚本、同一种归档。
- `Release` 工作流先在 Ubuntu 上跑 Rust 检查，再在 macOS 上构建、校验并归档 arm64 与 x86_64 两个包，最后发布。没有任何步骤读取签名密钥。
  macOS 作业用 `dtolnay/rust-toolchain@stable` 安装 Rust 和 `aarch64-apple-darwin`、`x86_64-apple-darwin` 两个 target，
  因为 `scripts/assemble-package.sh` 会按架构编译后端并放进包的 `bin/`（`universal` 用 `lipo -create` 合并）。
- CI 的 `package` 任务（macOS，草稿 PR 不跑）同样安装两个 target，构建并归档同样的两个架构，把 `dist/` 作为 `java-support-archives` 上传。
- SDK 固定在 `sdk.lock.json`，`scripts/verify-sdk-lock.sh` 检查 `Cargo.lock` 解析到的是同一个提交，加 `--download` 时再按 SHA-256 校验发布的 crate 包。
  后端依赖已发布的 Rust SDK 0.3.0（标签 `sdk-rust-v0.3.0`，提交 `db17c719`，插件函数协议 4）。
  SDK 从 0.2.0 起要求宿主提供 `litheLogging` 1：不提供的 Lithe 版本会拒绝启动后端。目前只有 Lithe 最新 `preview` 提供，稳定版还没有。
  日志的级别与类别见 README 的“日志”一节。

### 9.3 为什么仍是预发布

发布保持为预发布（`--prerelease`），商店的跟踪器会跳过预发布，所以目前不会上架。原因有三个：

1. 主仓库仍然以旧 ID `dev.lithe.plugin.java-support` 内置 Java Support（`Plugins/marketplace/official-plugins.json` 里的条目
   `releaseEnabled: false`，`bundledUntilPublishedVersion: "1.1.0"`）。
2. 包声明了 `backend` 2、`languageServers` 3（插件自有会话、`requestMethods`、`commands`）和 `pluginFunctions` 1，后端链接的 Rust SDK 0.3.0
   （插件函数协议 4）还要求宿主提供 `litheLogging` 1，不提供的 Lithe 版本会拒绝这个包或拒绝启动后端。这些接口来自主仓库的
   [#1745](https://github.com/1lck/Lithe-IDEA/pull/1745)、[#1746](https://github.com/1lck/Lithe-IDEA/pull/1746) 和中转 [#1742](https://github.com/1lck/Lithe-IDEA/pull/1742)，
   目前只有包含它们的 Lithe 最新 `preview` 提供，稳定版还没有，所以在 Lithe 稳定版提供它们之前，Release 保持预发布。
   （原来的第二个原因，即 `hostIntegration` 和短工具 ID `java` 只给 `dev.lithe.*` 或官方来源，已由主仓库 [#1763](https://github.com/1lck/Lithe-IDEA/pull/1763) 取消。）
3. Maven 与 Gradle 的新建项目模板和 Maven 原型生成器已经移交 lithe-plugin-maven 和 lithe-plugin-gradle（第 7 节）。在这两个插件接手之前发布新版本，
   “新建项目”会少掉 Maven 和 Gradle 选项，所以**在它们接手模板之前不要发布本插件的新版本**。

转为正式发布的条件：主仓库完成阶段 E（第 8.2 节），不再内置 Java Support，Lithe 稳定版已经提供 `backend` 2、`languageServers` 3、
`pluginFunctions` 1 和 `litheLogging` 1，并且 Maven、Gradle 插件已发布并带着模板。JDT 宿主集成由通用 API 取代是阶段 J2 的事（依赖 API-23），
不阻塞正式发布。之后在 `release.yml` 里去掉 `--prerelease`，商店自动上架稳定版。
下载的 ZIP 通过 设置 → 插件 的 ⚙ 菜单 → 从磁盘安装插件… 安装，但在此之前当前宿主会按上面的原因拒绝它。
阶段 E 还要退役内置的 `dev.lithe.plugin.java-support` 条目并把用户迁移到新 ID，见第 8.2 节。

### 9.4 运行时只读

插件包在运行时只读：JDT LS、Lombok、java-debug、vscode-java-test 运行器和 JDK 都在构建阶段固定来源、校验哈希后放进包里；
后端二进制 `bin/lithe-java-backend` 也在构建阶段由 `scripts/assemble-package.sh` 按架构编译后放进包里（`universal` 用 `lipo -create` 合并），
`scripts/verify-package.sh` 检查它是可执行文件、`lipo -archs` 与包的架构一致，且 `bin/` 里只有它一个文件。
运行时的状态只写宿主给的状态目录（后端不接收命令行参数，需要工作区状态目录时读宿主工作区上下文里的 `stateDirectory`），不写包目录，保证 Sparkle 增量更新的基线不变。
包里的 JDK 保留上游的 Apple 代码签名（macOS 只在有这个签名时才会启动它），`scripts/verify-package.sh` 检查这一点；
这是 JDK 自己的签名，不是插件发布者的签名。

## 10. 未完成事项

- 阶段 J2：语言服务器由后端启动（第 5.8 节），设置中转（API-25 / API-35）与运行/调试/测试（第 5.9 节）都在后端实现。
  仍未接：`java.configuration.runtimes` 的推送等 API-23 `toolchains.list`（后端的 `TODO(host-api: API-23 toolchains.list)`）；
  argfile 规划（`execution/launch_command.rs`）没有接到 `run/start`；TestNG 结果协议不解析；导航标记（父类/实现）与 CodeLens
  没有宿主 API；测试结果摘要是英文（等 API-37）；三个会话命令仍只回答 `NotSupported`。
- 运行/调试/测试的接管只由后端单元测试和 Core 清单校验验证，还要对着真实宿主（主仓库 Java 拆分分支）跑通：
  打开 Java 工程后出现行内 Run/Debug 标记、点击能启动并在 Run 窗口看到输出、调试能在断点停下、JUnit 测试结束后弹出摘要、
  堆栈行可点击跳转。
- Maven 与 Gradle 插件还没有接手新建项目模板（第 7 节）：lithe-plugin-maven 要带 `maven-archetype` 生成器、`pom.xml` 模板、Maven 图标和 `standard-*` 目录模板，
  lithe-plugin-gradle 要带 `gradle-kotlin`、`gradle-groovy` 模板和自己的一份 `standard-*` 目录模板。在它们接手之前不要发布本插件的新版本。
- 设置中转只由后端单元测试和桩中转测试验证，没有对着真实宿主跑过。变化通知（`languageImport.settingsChanged`）触发 `java.project.updateSettings`
  的完整链路，以及“没有提供方就不推送、沿用主仓库 `hostIntegration` 设置”的过渡规则，要在宿主合入 #1745、#1746 后用真实 JDT 验证。
- 没有真实 JDT 的端到端测试，也没有在 Linux 上实跑过（本机没有 Docker）。
- CI 的 `package` 任务构建的归档和正式发布的是同一种（同一组脚本、同一个文件名规则），但还没有真正跑过一次发布工作流：
  新 ID 下没有发布过任何版本，标签触发、预发布 Release 的创建和资产上传都没有被实际验证过。
- Windows 插件包没有做；`third_party/jdk/manifest.json` 里保留了 Windows 条目。
- 6 个 fixture 没有测试读取（第 7 节）。
- Spring Endpoints 面板的插件承载方式没有落点（第 3.3 节）。
