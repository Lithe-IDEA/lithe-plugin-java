# 第三方输入

插件包和独立运行环境均使用固定上游版本、HTTPS 地址和 SHA-256。仓库只保存清单、许可证和运行环境文件指纹，不提交上游发布二进制。

| 清单 | 内容 | 使用者 |
| --- | --- | --- |
| `jdtls/manifest.json` | Eclipse JDT LS 1.61.0、Lombok、java-debug、vscode-java-test 的固定下载输入；`minimumJavaVersion: 21` 是这个上游 JDT LS 的实际最低 Java 版本 | `scripts/prepare-jdtls.sh`、扩展打包、真实服务器测试 |
| `jdtls/runtime-files.json` | 该 JDT LS 原始归档的所有 114 个插件文件及五个平台配置模板的大小与 SHA-256 | `java.resolveRuntimes` 和启动前兼容性检查 |
| `../io.github.lithe-idea.java-support/plugin.json` 的 `managedRuntimes` | 与上述清单一致的 JDT LS 及六个平台完整 Microsoft JDK 25.0.5.0.2 的下载锁 | 宿主通用运行环境安装器 |

语言服务选择完整 JDK 25 是本插件的验证策略，不把 JDT LS 1.61.0 的上游最低要求误写成 25。项目的 JDK 和 source/target/release 与它分开。插件 ZIP 不包含 JDT LS / JDK；宿主安装时先检查可复用的外部或缓存安装，仅下载缺失依赖。

构建脚本把下载物放在被忽略的 `.artifacts/`，先校验 SHA-256 再使用。组装时用固定 JDT LS 的 `plugins/` 剔除它已包含的 Java Test 扩展包，并检查运行环境文件指纹一致。随包第三方许可证从上游复制，`NOTICE` 列出许可。运行环境按完整上游归档安装，保留其许可证与第三方 notices。

升级某个依赖时同时更新版本、不可变地址、SHA-256；JDT LS 还需更新运行环境 profile 与扩展兼容性验证，不因最新版本的下载可用就自动升级。对已校验解包目录执行：

```sh
node scripts/package/jdtls-runtime-profile.mjs .artifacts/jdtls --write
```

该命令只用于已审查的上游 pin 更新。正常 `prepare-jdtls.sh` 会比较而不会重写 profile。`backend/tests/repository_pins.rs` 检查 HTTPS、摘要格式、六平台覆盖、配置目录和 JDT LS 清单的一致性。上游来源和验证边界见 [运行环境说明](../docs/managed-runtimes.md)。
