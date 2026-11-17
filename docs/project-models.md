# 旧式与跨 IDE 工程模型（3.1.0）

没有 Maven/Gradle 构建文件的 Java 项目，常常带着其他 IDE 留下的工程结构：Eclipse WTP 的类路径容器、
MyEclipse 的 J2EE 库、IntelliJ IDEA 的 `.idea` 与 `.iml`，或者只有 `WEB-INF/lib` 的传统 Web 目录。
JDT LS 只认识 Maven、Gradle 和 Eclipse 的 `.project`，其余目录按“普通文件夹”导入，这些库和源码根会丢失，
`javax.servlet` 等也解析不到（[1lck/Lithe-IDEA#2063](https://github.com/1lck/Lithe-IDEA/issues/2063)）。
插件在不修改项目文件的前提下补上这部分。

## 由谁决定类路径

按以下顺序，第一个命中的接管，后面的不再参与：

1. Maven（`pom.xml`）或 Gradle（`build.gradle[.kts]`、`settings.gradle[.kts]`）：完全由构建工具导入，插件不叠加任何东西。
2. Eclipse `.project`：JDT 读取 `.classpath`。其中 JDT 自身不认识的容器由插件的 JDT LS 扩展包解析（见下文）。
3. IntelliJ IDEA：`.idea/modules.xml`，或基于文件的 `.ipr`。
4. 传统 Web 目录：含 `WEB-INF` 的 Web 根，或含 `META-INF/application.xml`/`EarContent` 的 EAR。
5. 普通文件夹：JDT 上游默认行为（`lib/**` 下的 JAR，源码根由 package 声明推断）。

识别只读项目文件：一次有界扫描（深度 6、最多 20,000 个条目、跳过隐藏目录和 `target`/`build`/`bin`/`out`/`node_modules` 等），
每个描述文件最多读取 2 MiB。设置“Java → 工程模型 → 自动识别工程模型”可以整体关闭；关闭后行为与 3.0.0 相同。

## 各模型补充的内容

| 来源 | 插件补充 | 写入位置 |
| --- | --- | --- |
| Eclipse WTP `org.eclipse.jst.j2ee.internal.web.container`（Web App Libraries） | `WEB-INF/lib` 下的 JAR，以及组件文件中部署到 `/WEB-INF/lib` 的工具模块 | JDT 内存中的类路径容器 |
| Eclipse WTP `org.eclipse.jst.j2ee.internal.module.container`（EAR Libraries） | 引用本模块的 EAR 的库目录（`application.xml` 的 `library-directory`，默认 `lib`）和 `MANIFEST.MF` 的 `Class-Path` | 同上 |
| Eclipse `org.eclipse.jst.server.core.container`（Server Runtime）、MyEclipse J2EE 1.3/1.4、Java EE 5/6/7 库 | Servlet 容器的 `lib` 下的 JAR | 同上 |
| Eclipse 类路径变量 `TOMCAT_HOME`、`CATALINA_HOME` | Servlet 容器的安装目录 | JDT 内存中的类路径变量 |
| IntelliJ IDEA 模块 | 生产与测试源码文件夹（`java.project.sourcePaths`）、模块库、项目库（`.idea/libraries`）、`jarDirectory` | 推给 JDT 的设置，仅内存 |
| IDEA 应用服务器库、Web facet | Servlet 容器的 JAR | 同上 |
| 传统 Web / EAR | `WEB-INF/lib/*.jar`、EAR 库目录、`MANIFEST.MF` 的 `Class-Path` | 同上 |

`.vscode/settings.json` 中已有的 `java.project.referencedLibraries`、`java.project.sourcePaths`、`java.project.outputPath`
始终优先：插件只在它们之后追加识别结果，保留其中的 `exclude` 和 `sources`；没有显式值时保留上游默认的 `lib/**`。

无法自动补全的条目会在“Java 库”设置页的“工程模型 → 未能解析”中逐项列出原因：IDEA 的全局库或应用库、
只有 IDEA 知道的路径变量、类文件夹（JDT 只接受 JAR）、工作区之外的源码文件夹、Eclipse 用户库（定义在 Eclipse 工作区里）、
不存在的文件。启动时同样以降级诊断计数记录：`launch.servlet_container_missing`、`launch.project_model_unresolved`、
`launch.project_model_truncated`。

## Servlet 容器

需要 Servlet API 的项目（Web 根、Server Runtime 容器、IDEA Web facet 或应用服务器库）按以下顺序选择容器：

1. 设置“Servlet 容器文件夹”。目录中没有 Servlet API 库（`lib/servlet-api.jar`，Tomcat 5.5 为 `common/lib`）时直接提示错误，不再回退。
2. 环境变量 `CATALINA_HOME`，然后 `TOMCAT_HOME`。
3. 常见安装位置：Homebrew（`opt/tomcat*/libexec`）、`/usr/local`、`/opt`、`/Library`、`/usr/share`、
   Windows 的 `Program Files\Apache Software Foundation` 和 `C:\`、用户主目录。优先选择项目点名的大版本
   （例如 `Apache Tomcat v9.0`），再按 `web.xml` 的 `javax`/`jakarta` 命名空间匹配，最后取最新版本。

容器路径属于本机，不会写入任何共享文件。

## 设置页与保存

“Java 库”设置页新增“工程模型”分组，只读显示：识别到的模型、自动识别的库（来源、所属模块、JAR 数）、IDEA 源码文件夹、
Servlet 容器及其来源、Eclipse 每个项目的容器与变量解析结果，以及未能解析的条目。只读行绑定到模型自己的描述文件
（`.idea/modules.xml`、`.project`、`WEB-INF/web.xml` 等），宿主的修订校验照常生效。

普通文件夹的 `.vscode/settings.json` 来源新增：

- “源码文件夹”：`java.project.sourcePaths`，留空时继承 IDEA 模块的源码文件夹。
- “将识别结果保存到此文件”：勾选并保存后，把识别出的库（不含本机 Servlet 容器路径）和 IDEA 源码文件夹写进
  `.vscode/settings.json`，供 VS Code 等其他编辑器和团队成员使用；取消勾选并保存则移除这些条目。

只有点击“保存”才会写文件；读取、启动和推送设置都不写项目文件。

## JDT LS 扩展包

Eclipse 容器和类路径变量只能在 JDT LS 进程内解析，插件为此自带一个扩展包
`jdtls-extensions/project-model/io.github.lithe_idea.java.projectmodel_<版本>.jar`，源码在 `jdtls-extension/project-model/`。

- 构建：`scripts/package/project-model-bundle.mjs` 在打包时以 `--release 17` 对照 `scripts/prepare-jdtls.sh` 准备的 JDT LS 编译，
  需要 JDK 21 或更高的 `javac`（`JAVA_HOME`，或用 `LITHE_JAVAC`/`LITHE_JAR` 指定）。条目按固定日期打包，同一源码与 JDK 产出相同字节。
  macOS（`scripts/assemble-package.sh`）、Linux（`scripts/package/build-linux.mjs`）、Windows（`scripts/package/build-windows.mjs`）三个入口都会构建，
  `scripts/verify-package.sh` 与 `backend/tests/repository_pins.rs` 校验它在包内。
- 加载：后端在启动参数的 `initializationOptions.bundles` 中列出它，与 Java Debug、Java Test 扩展相同。
- 运行时：只读取 JDT 设置 `java.lithe.projectModel`（后端推送的开关与 Servlet 容器）和项目文件；容器内容只存在于 JDT 的内存中，
  不写 `.classpath`、插件包或 JDT LS 安装目录。项目文件变化（JAR、组件文件、`.mymetadata`、`application.xml`、`MANIFEST.MF`）后自动重新计算。
- 命令 `lithe.java.projectModel.describe` 返回各 Eclipse 项目的容器、变量和缺失库的解析状态，供设置页展示。
- 生命周期：随插件版本发布，属于插件包的一部分，运行时只读；不下载、不缓存，也没有需要跨工作树复用的产物。

## 验证

- `cargo test --workspace`：识别、合并规则、设置页投影与保存的单元测试。
- `node scripts/compiler/test-project-models.mjs <插件包> [eclipse-wtp,legacy-web,idea]`：用打好的包和真实 JDT LS 编译三类工程
  （Eclipse WTP + Server Runtime + `TOMCAT_HOME`、只有 `WEB-INF/lib` 的传统 Web、非标准源码根和模块/项目库的 IDEA 项目），
  核对设置页内容、项目文件（JDT 自己在 `.project` 中添加的资源过滤除外）、插件包和 JDT LS 安装目录均未被修改；
  传统 Web 场景还会通过设置页保存识别结果，再从文件重新编译。关闭识别后三类工程都会编译失败，可作反向对照。
