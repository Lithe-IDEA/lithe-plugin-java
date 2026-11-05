# Java Support for Lithe

[Lithe](https://github.com/1lck/Lithe-IDEA) 的官方 Java 插件（`dev.lithe.plugin.java-support`）：
打包 Eclipse JDT Language Server、Lombok、java-debug、vscode-java-test 运行器和每个架构一份 Temurin JDK，
为 Java 项目提供补全、诊断、导航、运行/调试入口、测试运行和 Spring Boot / MyBatis 索引。

> English: The official Java plugin for Lithe. It bundles Eclipse JDT LS, Lombok, the java-debug and vscode-java-test
> runners and a per-architecture Temurin JDK, and provides Java language features, run/debug entry points, test runs
> and Spring Boot / MyBatis indexes. The Java rules live in a Rust domain crate and a plugin backend built on the
> published Lithe Rust plugin SDK. Maven and Gradle import settings come from the Maven and Gradle plugins through
> a function interface that Lithe Core relays between plugins.

> **进行中**：本仓库正在从 Lithe 主仓库拆分出来，目标是 Lithe 宿主（Rust Core 和 macOS 应用）不再包含任何 Java 专属逻辑，
> Java 的领域规则全部由本插件提供，宿主只提供通用插件接口。插件后端还没有接入宿主：`plugin.json` 暂不声明 `backend`，
> 要等 `MIGRATION.md` 第 6 节列出的宿主插件接口合并。在那之前 Lithe 仍然内置 Java Support，本仓库发布的版本标为预发布。

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
  所有宿主差异（存储、日志、Git、语言服务器通道）都是带 `TODO(host-api: API-NN)` 的占位。
- **插件后端**在 `backend/`：基于固定版本的 Rust SDK（`sdk.lock.json`），把每条领域命令注册为同名插件函数，
  并带着跨插件函数的调用方一侧。
- **跨插件函数**：启动 JDT LS 之前，后端通过 Core 中转发现所有导出 `languageImport.settings` 的插件（Maven、Gradle），
  逐个调用，只接受落在 JDT LS 导入设置命名空间里的键，合并后交给语言服务器。
  Maven 设置变化时调用 Java 导出的 `languageImport.settingsChanged`；Java 不会反过来调用正在调用它的插件。
  细节见 `MIGRATION.md` 第 5 节。
- **运行时只读**：插件包签名后在运行时只读。JDT LS、Lombok、java-debug、测试运行器和 JDK 都在构建阶段固定来源、校验哈希后放进包里，
  运行时状态只写宿主提供的状态目录。

## 安装

目前还不能从 Lithe Store 安装；Lithe 稳定版支持本插件所需的插件 API 之后，才会登记到商店索引（见 `MIGRATION.md` 第 9 节）。
在 Lithe 开发版中，可以通过 **设置 → 插件 → ⚙ → 从磁盘安装插件…** 选择本地构建的包目录安装
（发行版 Lithe 不加载未签名的开发包）。

## 开发

```text
dev.lithe.plugin.java-support/   # 插件包的静态部分：plugin.json、icons/、templates/，目录名必须等于插件 id
crates/java-domain/              # Java 领域规则（Rust），测试在各模块内和 tests/ 下
backend/                         # 插件后端进程（lithe-java-backend），基于 Lithe Rust 插件 SDK
fixtures/                        # 与主仓库共享的契约样例（JSON）
third_party/                     # JDT LS 与 JDK 的来源地址和 SHA-256
sdk.lock.json                    # 后端链接的 Rust SDK：标签、提交、crate 归档的 SHA-256
host-tools.lock.json             # 签名器和校验器所用的 Lithe 提交
scripts/                         # 组装、校验、归档插件包，构建宿主工具，检查 SDK 锁
.github/workflows/ci.yml         # PR：格式、lint、测试、SDK 锁，以及 macOS 上的开发包构建
.github/workflows/release.yml    # 推送 v<版本> 标签：测试后签名并发布 GitHub Release
MIGRATION.md                     # 主仓库路径到本仓库的映射、待宿主接口清单、主仓库删除清单
```

Rust 部分需要 Rust 工具链和 Cargo；打包脚本需要 macOS、`zsh`、`curl`、`ditto`，构建宿主工具时还需要 `host-tools.lock.json`
所对应提交要求的 Swift 工具链。

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/verify-sdk-lock.sh
```

所有测试都是有界、确定性的：不联网，不读全局时间，不占固定端口。

构建本地未签名开发包（会下载并校验 JDT LS 与 JDK，体积较大）：

```bash
scripts/build-package.sh --arch arm64          # 或 x86_64；打印包目录 .artifacts/package/<arch>/dev.lithe.plugin.java-support
scripts/archive-package.sh --package <上一步打印的目录> --arch arm64 --development-unsigned
```

签名发布包需要宿主工具（签名器和校验器，按 `host-tools.lock.json` 固定的 Lithe 提交构建）和发布私钥，只在发布工作流里进行：

```bash
scripts/host-tools.sh --require-pinned-swift   # 打印宿主工具所在目录
```

`plugin.json` 的完整格式见 lithe-plugin-docs 里的
[插件契约](https://github.com/Lithe-IDEA/lithe-plugin-docs/blob/main/contract/v1/README.md)。

## 发布新版本

1. 修改 `dev.lithe.plugin.java-support/plugin.json` 的 `version`，合入 `main`。
2. 推送同名标签（带 `v` 前缀，必须等于清单版本）：

   ```bash
   git tag v1.2.0 && git push origin v1.2.0
   ```

3. `Release` 工作流先在 Ubuntu 上跑 Rust 检查，再在 macOS 上构建并校验 arm64 与 x86_64 两个包；只有最后的签名与发布步骤拿
   `LITHE_PLUGIN_PACKAGE_PRIVATE_KEY`。发布资产是 `Lithe-Java-Support-<版本>-darwin-{arm64,x64}.zip` 和对应的 `.sha256`。
4. 在 Lithe 稳定版不再内置 Java Support 之前，发布保持为预发布（`--prerelease`）。

## 许可

插件自身的代码按 Apache License 2.0 发布（见 `LICENSE`）。插件包内的第三方组件各自保留原许可，见 `NOTICE`。
