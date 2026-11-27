# 普通 Java 与 Eclipse 编译设置与本地 JAR

在设置的“构建、执行、部署”中打开插件提供的编译器页，点击“读取编译设置”（构建工具会显示“使用项目工具读取编译设置”）。读取会使用项目工具，可能执行项目脚本或下载依赖。项目默认目标与模块覆盖分别显示；移除模块覆盖后继承项目配置。只有点击“保存”才写入 `.settings/org.eclipse.jdt.core.prefs`、普通项目的 `.vscode/settings.json` 或 Eclipse `.classpath`，外部修改会要求重新读取。

管理 JDK：在“构建、执行、部署”的工具链区使用“全局默认”，选择自动识别结果，或从磁盘添加多个 JDK，核对版本后保存；无需先打开项目。个别项目可切换到“当前项目”单独覆盖。已启动的进程需重启后采用新版本。JDK 是运行编译器的程序；目标字节码决定生成的 class 版本，两者分别设置。构建工具自己的显式 Java 环境或工具链配置仍按该工具的规则生效。

项目结构入口在插件的语言设置页，点击“读取项目结构”：Eclipse 工程按模块编辑源文件夹、输出文件夹、JRE 和依赖并写回 `.classpath`，Maven/Gradle 模块只读，详见 [module-structure.md](module-structure.md)；没有 `.classpath` 的普通文件夹在“引用的库”分组中选择文件并保存。每行一个路径，空格属于路径内容。JDT 的公开项目设置提供模块与实际值。JSONC、XML 和 properties 编辑器保留无关字段及注释；已有 JAR 排除和源码附件不被覆盖。Maven/Gradle 管理的项目使用各自插件的编译设置。

没有 Maven/Gradle 的旧式 Web、Eclipse WTP 和 IntelliJ IDEA 项目，插件会只读识别其工程模型并在内存中补全类路径，“Java 库”页的“工程模型”分组列出识别结果，详见 [project-models.md](project-models.md)。

运行时状态和暂存放在宿主提供的用户状态/临时目录；插件安装包始终只读。暂存文件由一次保存拥有，完成或取消时删除，不跨工作树共享。新后端与翻译由各平台构建入口打入同一版本包并校验。

## Compatibility

This version requires a desktop host advertising `sourceSettings: 1`. Older hosts retain their previous compatible plugin version. Read compiler settings under Build, Execution, Deployment; read the project structure on the language page. Save changes only the listed native sources and rejects a changed baseline. JDK selection and target bytecode are separate settings.

Tests in `scripts/compiler/` execute the real upstream engine, inspect class-file major versions and run code using an external JAR. HTML and JUnit timing reports are written to `.artifacts/test-stability/`; tests own and clean their temporary workspaces and processes.
