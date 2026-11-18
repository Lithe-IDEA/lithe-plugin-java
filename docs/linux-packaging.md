# Linux 构建与验证

`node scripts/package/build-linux.mjs arm64|x64` 在对应 Linux runner 上编译 Rust 后端，校验 ELF 架构，并从固定 SHA-256 的 Lombok、Java Debug 和 Java Test 输入组装包。插件 ZIP 不含 JDT LS、JDK 或符号链接。运行环境的固定来源和校验和写入清单，由宿主在首次安装或修复时独立准备；ARM64 读取 `config_linux_arm`，x64 读取 `config_linux`。

`node scripts/package/smoke-linux.mjs <zip>` 在原生 runner 上运行解压后的 SDK 后端，用 `scripts/prepare-jdtls.sh` 准备的 JDT LS 1.61.0 和完整 JDK 25 调用两次 `java.resolveRuntimes`，检查路径复用，再把批准的 `runtimes` map 交给 `java.resolveLaunch`。真实 JDT LS 执行 initialize/shutdown；完整插件、JDT LS 和 JDK 的文件、链接目标与权限快照必须不变。状态、Equinox 配置副本和索引位于测试临时用户目录；结束时清理进程组与目录。

既有 CI 的 JDK 21 可继续用于编译扩展。测试输入没有显式指定服务 Java、且 runner 没有 JDK 25 时，`prepare-test-jdk.mjs` 按同一清单 pin 在 `.artifacts/managed-jdk` 准备完整 JDK 25；这仅是构建/测试支持，不参与生产安装。配置 `LITHE_TEST_JDTLS_HOME`、`LITHE_TEST_JDTLS_JAVA` 可指定已准备的测试输入；显式服务 Java 必须为 25。项目扩展仍保留原有字节码基线，不因此修改项目 JDK。

宿主实际安装/解包验证由本仓库持有的 `scripts/package/test-host-managed.mjs` 在配套 host 检出上运行 Go overlay，调用真实 Core、SDK 后端与 `pluginservice.EnsureRuntimes`。输入是 SHA-256 已验证的官方完整归档；测试 HTTP transport 提供这些字节，不伪造解压结果，也不依赖每次联网。覆盖缓存准备、重复启动/插件更新零下载、只绑定服务工具、损坏 JDK 从已验证归档恢复。它需要不存在外部兼容运行环境的隔离 Linux x64 runner，具体命令见 [managed-runtimes.md](managed-runtimes.md)。

Linux 包面向 GNU/glibc 环境；原生 CI 使用 Ubuntu 24.04，不面向 musl/Alpine。原生 ARM64 和 x64 CI 的新流程必须各自在对应 runner 通过；本机 x64 结果不能替代 ARM64 验收。

## 资源与验证边界

构建输出位于 `dist/`，构建下载和测试状态位于 `.artifacts/`、`.cache/`、`target/` 或工具自身缓存。用户运行环境位于宿主的插件独立 `state/runtimes`，不写进插件安装版本目录。该缓存与插件更新分别管理；首次准备后，正常启动不重新下载同一 pin。日志、索引和 Equinox 可写配置仍使用宿主状态目录。

Linux 不使用 macOS 代码签名或 Sparkle。本次不改变既有 macOS 签名、Store 发布或更新基线；未发布新版本、Release 附件或 Store 条目。构建和自动化通过不等于原生窗口的全部交互验收。
