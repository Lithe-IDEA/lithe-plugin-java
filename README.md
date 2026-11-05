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
> Java 的领域规则全部由本插件提供，宿主只提供通用插件接口。插件后端还没有接入宿主：`plugin.json` 暂不声明 `backend`，
> 要等 `MIGRATION.md` 第 6 节列出的宿主插件接口合并。在那之前 Lithe 仍然内置 Java Support（旧 ID `dev.lithe.plugin.java-support`），
> 本仓库发布的版本标为预发布，原因见 `MIGRATION.md` 第 9 节。

## 功能

| 功能 | 说明 |
| --- | --- |
| 语言服务 | 用打包的 JDT LS（含 Lombok）提供补全、诊断、导航；JDK 随包提供，只用来运行语言服务器，项目自己的 SDK 仍由用户选择 |
| 运行与调试入口 | 识别 `main` 方法和测试方法，生成运行/调试标记 |
| 测试 | 发现 JUnit 测试项，生成测试选择器，并驱动 java-debug 与 vscode-java-test 运行器 |
| Spring Boot / MyBatis | 索引 Spring 端点与 MyBatis 映射，支持跳转 |
| 新建项目 | 三个声明式项目生成器：Java、Maven Archetype、Spring Boot |
| Maven / Gradle 工程 | 导入设置（settings 文件、本地仓库、offline、Gradle user home 等）由 Maven、Gradle 插件通过 `languageImport.settings` 提供，Java 不计算也不解析它们 |

## 工作方式

- **领域规则**在 `crates/java-domain`，不依赖宿主：命令的 JSON 载荷和错误码与主仓库 Core 一一对应。
  所有宿主差异（存储、Git、语言服务器通道）都是带 `TODO(host-api: API-NN)` 的占位；日志已接入宿主统一日志，见“日志”一节。
- **插件后端**在 `backend/`：基于固定版本的 Rust SDK（`sdk.lock.json`），把每条领域命令注册为同名插件函数，
  并带着跨插件函数的调用方一侧。
- **跨插件函数**：启动 JDT LS 之前，后端通过 Core 中转发现所有导出 `languageImport.settings` 的插件（Maven、Gradle），
  逐个调用，只接受落在 JDT LS 导入设置命名空间里的键，合并后交给语言服务器。
  Maven 设置变化时调用 Java 导出的 `languageImport.settingsChanged`；Java 不会反过来调用正在调用它的插件。
  细节见 `MIGRATION.md` 第 5 节。
- **运行时只读**：插件包安装后在运行时只读。JDT LS、Lombok、java-debug、测试运行器和 JDK 都在构建阶段固定来源、校验哈希后放进包里，
  运行时状态只写宿主提供的状态目录，不写包目录。

## 日志

后端把日志交给宿主的统一日志 `litheLogging` 1，不写 stderr。Rust SDK 0.2.0 要求宿主提供 `litheLogging` 1，
不提供的 Lithe 版本会拒绝启动后端；目前只有 Lithe 最新 `preview` 提供，稳定版还没有。

| 事实 | 级别 | 来源 | 内容 |
| --- | --- | --- | --- |
| 关键操作成功：`runConfig.selectJava`、`debug.javaTestLaunch`、`execution.planLaunchCommand`、`java.prepareLaunch`（`backend/src/lib.rs` 的 `KEY_OPERATIONS`） | INFO | SDK 自动 | 操作 ID、耗时 |
| 其余函数成功（编辑器驱动或启动时的读取，如 `java.runMarkers`、`java.codeVision`、`spring.index`） | 不记录 | `suppress_success_diagnostics`（`backend/src/lib.rs`） | — |
| 任一函数失败 | ERROR | SDK 自动 | 类别为 `pluginErrorCode`（如 `java.invalidRequest`）、耗时；不含消息与 `details` |
| 取消 | 不记录 | SDK | — |
| 列出 `languageImport.settings` 提供方失败、单个提供方失败或超时、答复不可用、答复含越界键 | WARN（`degraded`） | `backend/src/plugin_functions.rs` | 类别 `relay.providers_unavailable`、`relay.provider_failed`、`relay.provider_timed_out`、`relay.settings_rejected`、`relay.keys_rejected`，计数；不含提供方 ID |
| 插件存储清理失败 | WARN（`degraded`） | `crates/java-domain/src/host/diagnostics.rs`（`host::diagnostics`） | 类别 `storage.cleanup_failed.<阶段>`（阶段为 `jdt-configuration`、`jdt-settings`、`java-argfile-remove`），计数 |
| JDT LS 自身日志 | 上游，默认 WARNING | `plugin.json` 的 `-Dosgi.logfile=${logDir}/jdt.log` | 只写宿主提供的会话日志目录，单文件 1 MB、保留 2 个备份；用户可在设置里调 `logLevel` |

日志事实只带类别、计数和耗时，不含路径、源码、工具输出或用户内容。`${logDir}` 由宿主经可写存储适配器提供，不写插件包或 app bundle。

## 安装

Lithe 不再区分官方插件和普通插件，所有插件都按普通插件处理：本插件不带发布者签名（`plugin.json` 的 `vendor.signatureRequirement` 为 `none`）。
区别只在 [Lithe Store](https://github.com/Lithe-IDEA/Lithe-store) 怎样收录：Lithe-IDEA 组织里 `lithe-plugin-*` 仓库的稳定版（非预发布）
GitHub Release 由商店机器人自动跟踪并上架；组织之外的插件向 Lithe-store 提交 pull request，维护者审核后上架。

目前本仓库只发布预发布版，商店机器人会跳过，所以还不能从 Lithe Store 安装。当前版本的 Lithe 也会拒绝这个包：
它只允许 `dev.lithe.*` 插件使用 JDT 宿主集成（`languageServers[].hostIntegration`），只对带宿主验证过的官方来源的包接受短工具 ID `java`，
而本包两者都用到了；此外 Lithe 仍然内置旧 ID `dev.lithe.plugin.java-support` 的 Java Support。
具体原因和转为正式发布的条件见 `MIGRATION.md` 第 9 节。下载的 ZIP 通过 **设置 → 插件** 的 ⚙ 菜单 → **从磁盘安装插件…** 安装。

## 开发

```text
io.github.lithe-idea.java-support/   # 插件包的静态部分：plugin.json、icons/、templates/，目录名必须等于插件 id
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
包里不能有 `lithe-plugin-signature.json`，并检查打包的 JDK 保留着上游的 Apple 代码签名。归档不需要签名密钥，也不调用任何签名工具。

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
4. 发布保持为预发布（`--prerelease`）：Lithe Store 的自动跟踪会跳过预发布，原因有三个。目前的 Lithe 一是仍然内置旧 ID 的 Java Support，
   二是只允许 `dev.lithe.*` 插件使用 JDT 宿主集成、只对带官方来源的包接受短工具 ID `java`，
   三是后端链接的 Rust SDK 0.2.0 需要宿主提供 `litheLogging` 1，目前只有 Lithe 最新 `preview` 提供，稳定版还没有。
   等 Lithe 主仓库的阶段 E 完成（`MIGRATION.md` 第 8.2 节）、并且 Lithe 稳定版提供 `litheLogging` 1 后，去掉 `--prerelease`，
   商店就会自动上架稳定版。细节见 `MIGRATION.md` 第 9 节。

## 许可

插件自身的代码按 Apache License 2.0 发布（见 `LICENSE`）。插件包内的第三方组件各自保留原许可，见 `NOTICE`。
