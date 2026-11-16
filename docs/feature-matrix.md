# 功能验证矩阵

## 语言服务器来源（3.0.0）

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| 使用用户安装的 JDT LS（设置目录、`JDTLS_HOME`、`PATH` 上的 `jdtls`、Homebrew、mason.nvim、Scoop、VS Code Java 扩展） | implemented / verified | implemented / pending | implemented / pending | `backend/src/jdtls.rs` 单元测试；macOS arm64 本机用 Homebrew JDT LS 1.60.0 和固定的 1.61.0 跑 `scripts/compiler/test-live.mjs`（普通与 Eclipse 工程），核对插件包与 JDT LS 安装文件不变 |
| 用 `jdtls-runtime` 工具（JDK 21+）运行 JDT LS，低于 21 时报错 | implemented / verified | implemented / pending | implemented / pending | `backend/src/launch.rs` 单元测试；本机 JDK 21、26 均已验证，JDK 26 运行 JDT LS 时需宿主传入项目 JDK |
| 插件包不含 JDT LS 和 JDK | implemented / verified | implemented / pending | implemented / pending | `scripts/verify-package.sh`、`scripts/package/build-linux.mjs`；macOS arm64 包 11.3 MiB |

2026-11-16：Windows 与 Linux 的原生结果以 CI 和后续实机验证为准，本机结果不能推断。

## Linux 安装包

| 能力 | Linux ARM64 | Linux x64 | 验证入口 |
| --- | --- | --- | --- |
| 构建与安装包 | 已实现；[原生验证结果](https://github.com/Lithe-IDEA/lithe-plugin-java/pull/15/checks) | 已实现；[原生验证结果](https://github.com/Lithe-IDEA/lithe-plugin-java/pull/15/checks) | `.github/workflows/ci.yml`、[资源说明](linux-packaging.md) |
| 真实工具/语言服务器流程 | [原生验证结果](https://github.com/Lithe-IDEA/lithe-plugin-java/pull/15/checks) | [原生验证结果](https://github.com/Lithe-IDEA/lithe-plugin-java/pull/15/checks) | `scripts/package/` 和现有集成测试 |

跨架构本地包检查通过不等于原生工作流验收。既有平台验证记录保持不变。

## 原生编译设置与本地 JAR

| 能力 | macOS | Windows | 证据 |
| --- | --- | --- | --- |
| 项目默认目标、按模块字节码、继承与显式保存 | implemented / pending | implemented / pending | `scripts/compiler/` 真实引擎回归，检查 class major 52/55；原生完整 UI 验收仍 pending |
| 原生 JAR 引用与编译运行 | implemented / pending | implemented / pending | `scripts/compiler/` 编译外部 JAR 并运行引用它的程序；配置与包的只读边界见 native-settings.md |

2026-11-15：本地 macOS 自动化已通过，不将脚本通过等同于所有交互场景验收。Windows 的实机结果另随 PR 补充。
