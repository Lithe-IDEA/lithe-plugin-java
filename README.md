# Java Support for Lithe

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
> 后端目前承担 Maven/Gradle 导入设置的中转（见“工作方式”）。语言服务器仍然由宿主的 JDT 集成启动
> （`launch.kind` 为 `runtime`，带 `hostIntegration`）；改为后端启动、去掉 `hostIntegration`、由插件接管运行/调试/测试，
> 要等宿主提供 API-23（`toolchains.list`），清单见 `MIGRATION.md` 第 6 节和第 10 节。在那之前 Lithe 仍然内置 Java Support
> （旧 ID `dev.lithe.plugin.java-support`），本仓库发布的版本标为预发布，原因见 `MIGRATION.md` 第 9 节。
> Maven Archetype 生成器、Maven 与 Gradle 的项目模板已经移交 lithe-plugin-maven 和 lithe-plugin-gradle；
> 在这两个插件接手模板之前不要发布本插件的新版本，否则“新建项目”会少掉 Maven 和 Gradle 选项。

## 功能

| 功能 | 说明 |
| --- | --- |
| 语言服务 | 用打包的 JDT LS（含 Lombok）提供补全、诊断、导航；JDK 随包提供，只用来运行语言服务器，项目自己的 SDK 仍由用户选择 |
| 运行与调试入口 | 识别 `main` 方法和测试方法，生成运行/调试标记 |
| 测试 | 发现 JUnit 测试项，生成测试选择器，并驱动 java-debug 与 vscode-java-test 运行器 |
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
- **运行时只读**：插件包安装后在运行时只读。JDT LS、Lombok、java-debug、测试运行器、JDK 和后端二进制都在构建阶段固定来源、校验哈希（或由锁定的源码编译）后放进包里，
  运行时状态只写宿主提供的状态目录，不写包目录。

## 日志

后端把日志交给宿主的统一日志 `litheLogging` 1，不写 stderr。函数 ID 都是编译期常量，注册只会因 ID 写错、重复或超出 SDK
上限而失败，单元测试 `the_backend_builds_with_every_command_registered` 在 CI 里挡住这种构建；万一发生，进程在握手前以状态 1 退出，
由宿主记录启动失败（`backend/src/main.rs`）。Rust SDK（0.2.0 起，当前 0.3.0）要求宿主提供 `litheLogging` 1，
不提供的 Lithe 版本会拒绝启动后端；目前只有 Lithe 最新 `preview` 提供，稳定版还没有。

| 事实 | 级别 | 来源 | 内容 |
| --- | --- | --- | --- |
| 关键操作成功：`runConfig.selectJava`、`debug.javaTestLaunch`、`execution.planLaunchCommand`、`java.prepareLaunch`、`languageImport.settingsChanged`（`backend/src/lib.rs` 的 `KEY_OPERATIONS`） | INFO | SDK 自动 | 操作 ID、耗时 |
| 其余函数成功（编辑器驱动或启动时的读取，如 `java.runMarkers`、`java.codeVision`、`spring.index`） | 不记录 | `suppress_success_diagnostics`（`backend/src/lib.rs`） | — |
| 任一函数失败 | ERROR | SDK 自动 | 类别为 `pluginErrorCode`（如 `java.invalidRequest`；`languageImport.settingsChanged` 的 `relay.caller_missing`、`relay.context_missing`、`relay.invalid_params`）、耗时；不含消息与 `details` |
| 取消 | 不记录 | SDK | — |
| 列出 `languageImport.settings` 提供方失败、单个提供方失败或超时、答复不可用、答复含越界键、提供方导出的函数版本不符 | WARN（`degraded`） | `backend/src/plugin_functions.rs` | 类别 `relay.providers_unavailable`、`relay.provider_failed`、`relay.provider_timed_out`、`relay.settings_rejected`、`relay.keys_rejected`、`relay.provider_version_mismatch`，计数；不含提供方 ID |
| 取不到工作区根目录、设置推给 JDT LS 失败、单个工程的 profiles 更新失败、语言服务器状态通知格式不对 | WARN（`degraded`） | `backend/src/settings_session.rs` | 类别 `session.root_unavailable`、`session.push_failed`、`session.project_update_failed`、`session.state_invalid`，计数；操作 ID 为 `notification`（状态通知触发）或 `languageImport.settingsChanged` |
| 插件存储清理失败 | WARN（`degraded`） | `crates/java-domain/src/host/diagnostics.rs`（`host::diagnostics`） | 类别 `storage.cleanup_failed.<阶段>`（阶段为 `jdt-configuration`、`jdt-settings`、`java-argfile-remove`），计数 |
| JDT LS 自身日志 | 上游，默认 WARNING | `plugin.json` 的 `-Dosgi.logfile=${logDir}/jdt.log` | 只写宿主提供的会话日志目录，单文件 1 MB、保留 2 个备份；用户可在设置里调 `logLevel` |

日志事实只带类别、计数和耗时，不含路径、源码、工具输出或用户内容。`${logDir}` 由宿主经可写存储适配器提供，不写插件包或 app bundle。

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
