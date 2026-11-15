# 功能验证矩阵

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
