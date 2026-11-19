# 功能验证矩阵

## 工具链分组与设置搜索关键词（本次改动）

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| `toolDefinitions` 升至版本 `1`：项目 `JDK`（`java`）标为 `category: "runtime"`、`audience: "user"`，服务运行环境 `JDK for JDT LS`（`jdtls-runtime`）标为 `category: "runtime"`、`audience: "plugin"`，在宿主工具链设置页归入“插件运行环境”折叠区 | implemented / verified | implemented / pending | implemented / pending | `io.github.lithe-idea.java-support/plugin.json`、`backend/tests/repository_pins.rs`（`the_language_service_jdk_is_separate_from_the_project_jdk`）、`scripts/verify-contract.py`（针对 Core `9dea6aa6eaaf830c3568b647aa7daae5018b9727`）及宿主 Viva 无头窗口渲染验证（2026-11-18 macOS arm64） |
| 声明 `settingRegions`（`language` 与 `compiler`）中英文设置搜索关键词，支持在设置侧栏检索 `jdtls`、`javac`、`字节码`、`类路径`、`Java 编译器` 等直达 Java 语言与编译器设置页 | implemented / verified | implemented / pending | implemented / pending | `io.github.lithe-idea.java-support/plugin.json`、`io.github.lithe-idea.java-support/l10n/zh-Hans.json`、`backend/tests/repository_pins.rs`（`manifest_and_zh_hans_declare_setting_regions`）、`scripts/verify-contract.py`（针对 Core `0b6da2725ded74de7c3b62b49d12c1947dc0a38a`）及宿主 Viva 无头搜索定位验证（2026-11-18 macOS arm64） |
| 声明 `requires.ui: 0` 与 `ui.contribute` 权限，为 `language` 与 `compiler` 两个 `settingRegions` 提供 `settings.view` 实时画布预览（Java 语言页 4 张配置随草稿与界面语言联动卡片；Java 编译器页 3 层配置覆盖顺序图，响应窗口宽度 `width` 与 `resize`） | implemented / verified | implemented / pending | implemented / pending | `io.github.lithe-idea.java-support/plugin.json`、`backend/src/settings_view.rs`、`backend/tests/repository_pins.rs`、`scripts/verify-contract.py`（Core `0b6da2725ded74de7c3b62b49d12c1947dc0a38a`）、`e2e/host/desktop/internal/settingswindow/plugin_native_source_test.go` 与 `scripts/compiler/visual.mjs` 经 Core `ApplyPluginViewResult` 与 Viva 无头深浅色中英文截图验证（2026-11-19 macOS arm64） |

## 自动准备语言服务环境

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| 完整 JDK 25 与固定 JDT LS 分别声明，已有依赖复用，只下载缺失部分 | implemented / pending | implemented / pending | x64 verified；ARM64 pending | `managed_runtimes` 单元测试覆盖四种存在组合；真实宿主安装结果见 [运行环境说明](managed-runtimes.md) |
| JDT LS 精确 114 文件 + 当前平台模板校验，拒绝混装/残缺；完整 JDK 的版本、OS、CPU、java/javac 执行文件检查 | implemented / pending | implemented / pending | x64 verified；ARM64 pending | 六平台文件布局与 ELF / Mach-O / PE 头单测；Linux 用官方归档及真实 SDK hook |
| 服务 JDK 25 独立于项目 JDK；显式选择优先，再复用受管缓存，最后自动发现 | implemented / pending | implemented / pending | x64 verified；ARM64 pending | `backend/src/managed_runtimes.rs`、`repository_pins.rs`；真实 launch 只绑定 `jdtls-runtime` |
| 插件 ZIP 无 JDT LS/JDK；批准的 runtimes map 启动真实 JDT，包/运行环境保持只读 | implemented / pending | implemented / pending | x64 verified；ARM64 pending | `scripts/package/smoke-linux.mjs` 的完整快照与 initialize/shutdown；[Linux 验证](linux-packaging.md) |

2026-11-18：官方完整 JDK 六平台 URL / SHA-256 与归档根目录已核对；实际机器运行仅覆盖 Linux x64，不能用文件头单测或下载可用性宣称其他平台原生通过。Windows ARM64 的运行环境声明不代表已新增 ARM64 宿主或插件发布包。此前 3.0.0 的 macOS 外部 JDT LS 1.60.0 / JDK 21 验证属于旧行为；当前兼容性锁只接收 JDT LS 1.61.0 + 完整 JDK 25。

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

## Eclipse WTP Web Facet 版本编辑

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| Java 项目结构中的 Web Facet 独立原生来源、按 jst.java 限定可选版本、版本读写与修改前 revision 校验；无法安全编辑的 Facet 文件只跳过该来源并上报一次降级 | implemented / verified | implemented / pending | implemented / pending | `backend/src/web_facet.rs` 单元测试；`scripts/compiler/test-module-structure.mjs` 用真实 JDT LS 1.61 覆盖读取、不合规文件、版本限制、保存，以及所有来源和字段 ID 符合 Core 规则；`scripts/compiler/visual.mjs` 在固定宿主上经 Core `ReadSourceSettings` 渲染 Web Facet 页，截图见 `docs/assets/module-structure/web-facet-*.png`。2026-11-18 macOS arm64 本机通过；Linux 由 CI 的同一脚本覆盖；Windows 与原生窗口点按待实机验收 |
| 编辑 WTP .component、IDEA .iml Web Facet、WTP 服务器配置 | not-implemented | not-implemented | not-implemented | 单文件安全修改，不覆盖其他 IDE 模型 |

## JSP / EL 语法着色

| 能力 | macOS | Windows | Linux | 证据 |
| --- | --- | --- | --- | --- |
| JSP/JSPF/Tag/Tagf 识别；在宿主 HTML 语法之上注入 JSP 指令、脚本片段、注释、EL 和标签库标签着色；注释与括号设置 | implemented / verified | implemented / verified | implemented / verified | `grammars/jsp.tmLanguage.json`；`scripts/grammar/host-jsp.mjs` 用宿主 TextMate 分词器（宿主 commit 见 `grammar-host.json`）逐项检查 24 处着色，2026-11-18 macOS arm64 本机与 CI Linux 通过。分词器与平台无关；原生编辑器窗口内的实际显示待实机验收 |
| JSP/EL 语义补全、诊断、页面断点 | not-implemented | not-implemented | not-implemented | JDT LS 仅为 Java 服务，未来需 JSP 专属语言服务 |
