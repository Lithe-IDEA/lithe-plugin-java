# 功能验证矩阵

## 语言服务器来源（3.0.0）

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| 使用用户安装的 JDT LS（设置目录、`JDTLS_HOME`、`PATH` 上的 `jdtls`、Homebrew、mason.nvim、Scoop、VS Code Java 扩展） | implemented / verified | implemented / pending | implemented / pending | `backend/src/jdtls.rs` 单元测试；macOS arm64 本机用 Homebrew JDT LS 1.60.0 和固定的 1.61.0 跑 `scripts/compiler/test-live.mjs`（普通与 Eclipse 工程），核对插件包与 JDT LS 安装文件不变 |
| 用 `jdtls-runtime` 工具（仅匹配 JDK 21+，含 VS Code / Cursor `redhat.java` 自带 JRE）运行 JDT LS，低于目标 JDT LS bundle 要求的最低 Java 版本（如 21 或 25）时报错 | implemented / verified | implemented / pending | implemented / pending | `backend/src/jdtls.rs`、`backend/src/launch.rs`、`backend/tests/repository_pins.rs` 单元测试；本机 JDK 21、25、26 均已验证，JDK 26 运行 JDT LS 时需宿主传入项目 JDK |
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

## 旧式与跨 IDE 工程模型（3.0.1）

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| Eclipse WTP/MyEclipse 容器（Web App Libraries、EAR Libraries、Server Runtime、Java EE 库）与 `TOMCAT_HOME`/`CATALINA_HOME` 变量 | implemented / verified | implemented / pending | implemented / pending | `jdtls-extension/project-model`；macOS arm64 本机用固定的 JDT LS 1.61.0 跑 `scripts/compiler/test-project-models.mjs` 的 `eclipse-wtp` 场景 |
| 传统 Web（`WEB-INF/lib`）、EAR 库目录与 `MANIFEST.MF` Class-Path | implemented / verified | implemented / pending | implemented / pending | `backend/src/project_model/web.rs` 单元测试；`legacy-web` 场景 |
| IntelliJ IDEA 模块源码根、模块库、项目库（只读导入） | implemented / verified | implemented / pending | implemented / pending | `backend/src/project_model/idea.rs` 单元测试；`idea` 场景 |
| Servlet 容器：设置、环境变量、常见位置自动查找 | implemented / verified | implemented / pending | implemented / pending | `backend/src/project_model/servlet.rs` 单元测试，含 Windows 安装位置 |
| “Java 库”页的工程模型只读分组、源码文件夹与“保存识别结果” | implemented / verified | implemented / pending | implemented / pending | 后端投影与保存经 `test-project-models.mjs` 真实验证；`scripts/compiler/visual.mjs` 用宿主 Viva 无头窗口渲染三类工程的真实投影（中英、明暗），截图见 `docs/assets/project-models/` |

2026-11-17：macOS arm64 本机自动化通过（三类工程编译成功，关闭识别后均失败；插件包、JDT LS 安装与项目文件不变）。Windows、Linux 结果由 CI 与后续实机验证补充。


## 项目结构：模块、标签页与依赖（4.0.0）

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| 模块侧栏：Eclipse 工程可编辑，Maven/Gradle 模块只读（JDT 解析的类路径），按文件夹嵌套 | implemented / verified | implemented / pending | implemented / pending | `backend/src/module_structure.rs`、`backend/src/project_settings.rs`；`scripts/compiler/test-module-structure.mjs` |
| 源文件夹（测试标记）、输出文件夹、JRE 执行环境写回 `.classpath` | implemented / verified | implemented / pending | implemented / pending | 单元测试 `module_structure::tests`；`test-module-structure.mjs` 保存后 JDT 编译到新输出文件夹 |
| 依赖：JAR/ZIP、类文件夹、库容器、模块依赖，导出与仅测试，保留源码附件、注释和未显示条目 | implemented / verified | implemented / pending | implemented / pending | 同上；`test-live.mjs` 的 Eclipse 模式经依赖列表添加外部 JAR 并编译运行 |
| 项目结构页无头截图 | implemented / verified | — | — | `scripts/compiler/visual.mjs` 渲染真实投影（中英、明暗），截图见 `docs/assets/module-structure/` |

2026-11-17：macOS arm64 本机用 JDT LS 1.61.0 自动化通过。需要宿主声明 `sourceSettings` 1（[1lck/Lithe-IDEA#2087](https://github.com/1lck/Lithe-IDEA/pull/2087)）；原生文件选择器、Windows 与 Linux 结果由实机验收和 CI 补充。

## JSP / EL 语法着色

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| JSP/JSPF/Tag/Tagf 识别；在宿主 HTML 语法之上注入 JSP 指令、脚本片段、注释、EL 和标签库标签着色；注释与括号设置 | implemented / verified | implemented / verified | implemented / verified | `grammars/jsp.tmLanguage.json`；`scripts/grammar/host-jsp.mjs` 用宿主 TextMate 分词器（宿主 commit 见 `grammar-host.json`）逐项检查 24 处着色，2026-11-18 macOS arm64 本机与 CI Linux 通过。分词器与平台无关；原生编辑器窗口内的实际显示待实机验收 |
| JSP/EL 语义补全、诊断、页面断点 | not-implemented | not-implemented | not-implemented | JDT LS 仅为 Java 服务，未来需 JSP 专属语言服务 |
