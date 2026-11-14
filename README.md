# Java Support for Lithe

> **Viva 预览版 Java 支持（1.1.3）**：清单已迁到当前契约 0，后端使用 Core
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

1.1.3 面向包含主仓库 [#1884](https://github.com/1lck/Lithe-IDEA/pull/1884) 的配套 Viva preview（客户端修订 `607f00967d70a7017d49805183d0d2e6ffdf1c0d` 或更新版本）。API / 能力与传输版本以 `plugin.json`、`sdk.lock.json`
和固定 Core 中的[插件资料](https://github.com/Lithe-IDEA/Lithe-core-rust/tree/65cf82343c5e315aae2e335b2e8f0217b703e89a/shared/plugins)为准。
`MIGRATION.md` 保留迁移历史，不能用其中旧宿主、旧 API 数字推断当前兼容性。

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
上述集成结果不能替代这些日志验收。生成器、JUnit/TestNG 行内运行器、Spring/MyBatis 未在本轮执行；Gradle 扩展结果见下表。

无头工作台结果不能视为原生窗口、Linux、任意复杂依赖、私有仓库或完整传统 Java EE 验收。
Windows 长 classpath 修复只覆盖 classpath，长 module-path 仍见 [#7](https://github.com/Lithe-IDEA/lithe-plugin-java/issues/7)。

### 本轮扩展验证

macOS arm64 的真实插件与 Viva 无头工作台已覆盖：

| 流程 | 结果与边界 |
| --- | --- |
| 普通 Maven + 第三方 JAR | 编译、补全、运行、断点变量、继续/停止、编译错误阻止旧 class |
| Gradle 7.6.4 Wrapper，两模块，中文空格目录 | Build 模型、compileJava、JDT 补全和同一 Run/Debug 流程；项目使用 Java 8 |
| 普通源码、外部 JAR、Eclipse 多源码根、Ant 目录 | JDT 编译后核对 major 52，并在 Java 8 实际运行/调试；没有执行 Ant target |
| Maven WAR / Tomcat 9.0.82 | 真实 JSP scriptlet、EL、JSTL、include HTTP 响应，以及 Servlet 断点/变量；未验收 JSP 源码补全或 JSP 断点 |
| Commons Text 1.9，`cb85bed468e99d34b88d0c81fe20eb3b1615660e` | 原 POM 不变，加入带许可证的 API 验证入口，Maven/补全/运行/断点/继续停止通过，36.25 秒 |

Windows 验证结果随原生报告另行记录，不能由上述 macOS 结果推断。ARM Windows 虚拟机上的 x64 工具经过系统仿真，耗时不能当成原生 AMD64 性能。
原生窗口、导航/虚拟源码、JUnit/TestNG 行内运行器、Spring/MyBatis、日志窗口、启动中取消和每一种失败恢复仍未完整验收。

当前真实旧项目限制：

- Lanterna 3.1.1 原多版本 JAR 配置：项目 JDK 11 下 Maven/补全通过，JDT 要求启用 release，运行前构建失败，见 [#1882](https://github.com/1lck/Lithe-IDEA/issues/1882)。
- JSQLParser 3.0：Maven 已生成 JavaCC 源码，JDT 模型仍缺相关类型，Run/Debug 未通过，见 [#1881](https://github.com/1lck/Lithe-IDEA/issues/1881)。
- jOOL 0.9.14 的 Java 8 子项目：Maven compile/补全通过，ECJ 的泛型捕获转换报错，不能算完整支持，见 [#1883](https://github.com/1lck/Lithe-IDEA/issues/1883)。

固定同一 Viva 测试二进制、Core、Java 17 和工程，关闭虚拟机，按 AB/BA 交错运行六对原始/最终真实 ZIP：12 轮全部通过。
中位数如下；Maven 依赖与系统文件缓存保留，每轮重新安装插件并新建工作区。

| 指标 | 原始 ZIP | 最终 ZIP | benchstat |
| --- | ---: | ---: | --- |
| 打开 Java 文件到动作可用 | 6.493 s | 6.421 s | 无显著差异，p=0.394 |
| 首次补全 | 75.98 ms | 74.46 ms | 无显著差异，p=0.699 |
| 热补全（每轮 12 次的中位数） | 11.76 ms | 11.87 ms | 无显著差异，p=1.000 |
| JDT 就绪后 RSS | 793.25 MiB | 793.79 MiB | 无显著差异，p=0.589 |
| JDT 流程结束 RSS | 816.13 MiB | 817.40 MiB | 无显著差异，p=0.699 |

Windows x64 在 ARM Windows 11 虚拟机中验证：Maven 64.77 秒、Gradle 104.91 秒（含真实配置回读且中转无超时）；
其他同一修复链路的原生结果含普通源码 52.80 秒、外部 JAR 49.46 秒、Ant 布局 55.70 秒、Eclipse 多源码根 47.11 秒、
长 classpath 48.70 秒、JSP/Servlet 74.70 秒。它们覆盖同一补全/运行/调试/停止/拒绝旧 class 流程，安装包哈希不变。
Windows 的三个通用工具入口单元回归与 Java 8 配置后端测试也通过。虚拟机已关闭。
这些结果不代表原生窗口、安装器、签名、ARM64 发布包或持续压力测试验收。

Java 工具定义的 JAR 入口模板由 Core 的既有工具规划接口实例化；宿主先验证包内入口路径。
Gradle 后端只提供自己的堆选项，`-jar` 与入口位置来自 Java 插件的工具定义，裸 JVM 参数的 Run/JDT 启动保持原有流程。
配套 Viva 必须包含工具入口接线；只有构建任务成功不能证明配置后端正常，真实回读与中转检查不可省略。

GC 和初始堆的隔离试验没有获得可靠收益，因此保留 Parallel GC、Xms256m / Xmx2048m；没有提交试验用 G1/Serial/ZGC 或 JDT 1.63 / Java 25 包。
内存报告中的 JDT 进程树 RSS 是观测时点，不是峰值、PSS 或原生客户端总内存，不能把测试驱动内存变化说成产品内存下降。

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

- Java 专属的 URI、JDT 初始化、编译/运行/调试和长 classpath 规划属于本插件及成熟上游；宿主只提供通用服务。
  Maven / Gradle 自己解析配置，通过 `languageImport.settings` 接口提供 JDT 导入设置，Java 不维护第二套构建模型。
- `backend/src/launch.rs` 定位已打包的 JDT、Lombok、java-debug、java-test 和运行语言服务器的 JDK。
  宿主传入用户所选项目 JDK 的安装事实，插件据此设置 JDT 运行时；随包 JDK 不代替项目的目标版本。
- 初始化就绪不等于索引完成。JDT 公开的 `progressReportProvider` 扩展将完成事件发到已声明的
  `language/progressReport` 通道。只有动作查询发生瞬态失败才在完成边界刷新；同时处理完成与超时倒序，
  每个失败周期最多自动刷新三次，成功重置预算，工作区/服务器代际隔离旧结果，关闭后删除状态。不增加空闲轮询。
- Windows 使用原生 `java.exe`，文件 URI 由 `url` crate 处理，包路径通过 `dunce` 去除可安全简化的 verbatim 前缀，
  避免 Lombok 的 `-javaagent` 加载失败。过长 classpath 复用领域层的 manifest JAR 规划，适用于 Java 8 及更新版本；
  ASCII 百分号编码覆盖中文/空格，宿主写入临时 JAR 并在运行结束删除。长 module-path 尚未覆盖。
- 已安装插件包始终只读。JDT 配置副本、索引、日志、锁和临时文件写到宿主提供的用户状态/缓存/临时目录；
  插件包、app bundle 和 Windows 安装目录不作为可写目标。构建期固定来源、校验资源哈希，运行期不下载。

## 日志

后端把日志交给宿主的统一日志 `litheLogging` 0，不写 stderr。函数 ID 都是编译期常量，注册只会因 ID 写错、重复或超出 SDK
上限而失败，单元测试 `the_backend_builds_with_every_command_registered` 在 CI 里挡住这种构建；万一发生，进程在握手前以状态 1 退出，
由宿主记录启动失败（`backend/src/main.rs`）。当前固定的 Rust SDK 0.4.0 要求宿主提供 `litheLogging` 0，
不提供的 Lithe 版本会拒绝启动后端；本次验证范围以本文列出的配套 Viva 检出为准。

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
| 运行动作失败 | ERROR | SDK 自动 | `backend/src/run.rs` 的 `RUN_ERROR_CODES`：`run.invalid_params`、`run.context_missing`、`run.workspace_unavailable`、`run.language_server_failed`、`run.build_failed`、`run.no_tests`、`run.port_unavailable`、`run.host_rejected`、`run.classpath_failed` |
| 动作注册、上下文读取或测试结果摘要降级 | WARN | SDK `degraded` | `RUN_CATEGORIES`：`run.actions_registration_failed`、`run.contexts_unavailable`、`run.result_summary_failed` |
| 冷索引动作查询延迟/恢复、自动刷新耗尽 | WARN | SDK `degraded` / `recovered` | `run.actions_query_deferred`、`run.actions_query_exhausted`、`run.actions_progress_limit`，每个失败周期记录延迟及恢复/耗尽，不含工作区内容 |
| 宿主启动 JDT LS、java-debug、项目 JVM | 宿主决定 | 声明式贡献/Run 生命周期 | 插件不重复发送启动终态；上游 stderr 由宿主按 DEBUG 接收 |
| JDT LS 自身日志 | 上游，默认 WARNING | `plugin.json` 的 `-Dosgi.logfile=${logDir}/jdt.log` | 只写宿主提供的会话日志目录，单文件 1 MB、保留 2 个备份；用户可在设置里调 `logLevel` |

日志事实只带类别、计数和耗时，不含路径、源码、工具输出或用户内容。`${logDir}` 由宿主经可写存储适配器提供，不写插件包或 app bundle。
Rust 后端的通道是 `plugins/io.github.lithe-idea.java-support/host`；JDT LS、java-debug 与项目 JVM
分别由宿主的语言服务器、调试和 Run 通道管理。只有 JDT LS 使用原生日志文件。

## 安装

本分支需要包含配套接线修复的 Viva preview。使用真实 ZIP 从客户端的插件安装入口安装，
同时安装兼容当前契约的 Maven / Gradle 插件。本轮使用 Maven 已发布的 `v0.0.1` 预发布包，
Gradle 使用未发布的 `1.0.0` 开发包；不能用旧稳定宿主的安装要求推断兼容性。
目前没有发布新 Release、tag 或更新市场条目。

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

Rust 部分需要 Rust 工具链和 Cargo；打包脚本需要 macOS、Node.js 22、`zsh`、`curl`、`ditto`，不需要 Xcode 或 Swift 工具链。
打包脚本会用 `cargo build --release --locked -p lithe-java-backend --target <triple>` 编译后端，所以还要安装两个 Apple target：
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`（`--arch arm64` 用前者，`x86_64` 用后者，`universal` 两个都编译再用 `lipo -create` 合并）。

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/verify-sdk-lock.sh
```

Rust 单元测试使用有界资源；真实 Viva 集成测试需要显式 ZIP、JDK、构建工具输入，外部依赖首次解析可能联网，不属于离线单元测试。

构建并归档插件包（会下载并校验 JDT LS 与 JDK，体积较大）。本地开发、CI 和发布使用同一组命令，产出同一种归档：

```bash
scripts/build-package.sh --arch arm64          # 或 x86_64；打印包目录 .artifacts/package/<arch>/io.github.lithe-idea.java-support
scripts/archive-package.sh --package <上一步打印的目录> --arch arm64   # 写入 dist/Lithe-Java-Support-<版本>-darwin-{arm64,x64}.zip 和对应的 .sha256
```

`build-package.sh` 会在输出包目录前校验它；`archive-package.sh` 把归档解开后再校验一次。单独校验一个包目录用
`scripts/verify-package.sh --package <目录> --arch arm64|x86_64|universal`：它要求 `plugin.json` 的 `vendor.signatureRequirement` 为 `none`，
包里不能有 `lithe-plugin-signature.json`，`bin/` 里只能有 `backend.launch.path` 指向的后端二进制，它必须可执行且包含请求的架构（`lipo -archs`），
并检查打包的 JDK 保留着上游的 Apple 代码签名。归档不需要签名密钥，也不调用任何签名工具。

`plugin.json` 的完整格式只在固定 Core 的 `shared/plugins/contract/v1/`，不在本仓库维护副本。

### Windows 包

在 Windows 从同一源码以 `--locked` 编译 x64 后端后，可在 macOS 组装用于原生验证的包：

```sh
node scripts/package/build-windows.mjs /path/to/lithe-java-backend.exe
```

脚本验证 x64 PE 与锁定 JDK 的 SHA-256，复用 JDT 构建入口，写入 `.artifacts/` 并清理组装临时目录。
发布工作流在 Windows 从同一标签构建并测试 x64 后端，静态链接编译器运行库，再通过此入口组装 ZIP；本地验证复用相同入口。发布前用成熟的 PE 检查工具拒绝额外的 VC++/MinGW 运行库 DLL 依赖，保持后端无需另装编译器运行库。包不带发布者签名，不含安装器，打包成功也不能代替 Windows 运行验收。

## 发布新版本

构建和发布由现有工作流执行，正式发布前按 Lithe 的版本规则决定版本并核验兼容宿主。
当前依赖 Viva preview，只能作为预发布。推送匹配版本的标签后，工作流为 macOS arm64/x64 与 Windows x64 生成 ZIP 和 SHA-256，并创建草稿预发布；维护者下载核对后再公开草稿。发布说明必须位于 `docs/releases/v<版本>.md`，缺失则终止发布。
不得覆盖已发布 ZIP，也不得将本地隔离试验的 JDT/JDK 资源当作生产升级：本轮生产仍固定 JDT LS 1.61 和 Temurin 21。

## 许可

插件自身的代码按 Apache License 2.0 发布（见 `LICENSE`）。插件包内的第三方组件各自保留原许可，见 `NOTICE`。

### 随包许可

两端构建都会复制自身 LICENSE/NOTICE，并用 Cargo 锁定的非 dev 构建图收集 Rust 依赖的许可，写入 licenses/rust 和不含本机路径的哈希清单。SDK 沿其 Git 检出查找共享许可；crate 未携带主许可时，只接受 third_party/rust-notices 中与 crate VCS 修订及 SHA-256 匹配的固定文本。tree-sitter 内嵌的 Unicode 许可一并保留。缺少许可或来源不匹配会终止打包；不在运行期下载，不跨工作树共享生成目录。
