# Linux 构建与验证

`node scripts/package/build-linux.mjs arm64|x64` 在对应 Linux runner 上编译 Rust 后端，校验其 ELF 架构，再从固定 SHA-256 的 Temurin JDK、JDT LS、Lombok、Java Debug 和 Java Test 输入组装包。ARM64 使用 `config_linux_arm`，x64 使用 `config_linux`。JDK 上游符号链接仅在构建时展开，插件 ZIP 不含链接。完整许可证与反编译补丁校验沿用已有流程。归档解压后再次检查清单、JDK/后端/VM 架构、执行权限和配置。

`node scripts/package/smoke-linux.mjs <zip>` 在原生 runner 上运行解压后的后端握手与 `java.resolveLaunch`，启动包内 Java 和 JDT LS，执行 initialize/shutdown，并对比整个安装目录的文件内容与权限。状态目录和 Equinox 配置副本在临时用户目录；结束时清理进程组和目录。

## 资源与验证边界

所有下载、解压、编译与归档都属于本插件仓库的构建阶段。`dist/` 为输出包；`.artifacts/`、`.cache/`、`target/` 或工具自身缓存为本地构建状态，不跨工作树共享可变状态。校验过的下载以固定来源与摘要识别，提取结果不作为可信下载源。运行时只读取已安装插件，缓存、日志、索引和工作区状态归用户可写目录。Linux 不使用 macOS 代码签名或 Sparkle；本次不改变 macOS 包签名与更新基线。

原生 ARM64 和 x64 验证由 `.github/workflows/ci.yml` 执行；本机的交叉编译与归档检查不能替代原生运行结果。只有目标平台 CI 成功后才能宣称该平台通过。发布新版本后，使用实际 Release 附件的 SHA-256 更新 Store，保留所有旧版本与既有平台。
