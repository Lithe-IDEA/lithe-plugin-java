# Java Support for Lithe

[Lithe](https://github.com/1lck/Lithe-IDEA) 的官方 Java 插件（`dev.lithe.plugin.java-support`）：
打包 Eclipse JDT Language Server、Lombok、java-debug、vscode-java-test 运行器和每个架构一份 Temurin JDK，
为 Java 项目提供补全、诊断、导航、运行/调试入口、测试运行和 Spring Boot / MyBatis 索引。

> English: The official Java plugin for Lithe. It bundles Eclipse JDT LS, Lombok, java-debug, the vscode-java-test
> runner and a per-architecture Temurin JDK, and provides Java language features, run/debug entry points, test runs
> and Spring Boot / MyBatis indexes. This repository is being split out of the Lithe main repository; see
> `MIGRATION.md` on the porting branch for status.

本仓库正在从 Lithe 主仓库拆分出来，目标是 Lithe 宿主（Rust Core 和 macOS 应用）不再包含任何 Java 专属逻辑：
Java 的领域规则全部由本插件提供，宿主只提供通用插件接口。

## 许可

插件自身的代码按 Apache License 2.0 发布（见 `LICENSE`）。插件包内的第三方组件各自保留原许可，见 `NOTICE`。
