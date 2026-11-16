# 第三方输入

`third_party/` 只记录插件包需要的上游输入，每个组件一个 `manifest.json`：上游版本、HTTPS 下载地址和 SHA-256。

| 清单 | 内容 | 使用者 |
| --- | --- | --- |
| `jdtls/manifest.json` | 随包的 Lombok、java-debug、vscode-java-test 扩展，以及测试用 Eclipse JDT Language Server 的版本、下载地址和 SHA-256；`minimumJavaVersion` 是运行 JDT LS 的最低 Java 版本，与后端的 `MINIMUM_SERVER_JAVA_VERSION` 一致 | `scripts/prepare-jdtls.sh` |

插件包不带 JDT LS 和 JDK：插件运行用户自己安装的 JDT LS 和 JDK 21+（`backend/src/jdtls.rs`）。清单里的 JDT LS
只用于构建和测试：组装时据它的 `plugins/` 剔除 JDT LS 已自带的 Java Test 扩展包，集成测试把它当作用户的安装来启动。

规则：

- 下载物放在被 `.gitignore` 忽略的 `.artifacts/` 下，先校验 SHA-256 再使用；缓存校验失败时删除重下。
  不要把 jar、JDK 或其他发布二进制提交到仓库。
- 升级组件时同时改版本、地址和 SHA-256，并在 PR 里写明上游发布页。`cargo test` 会检查每个地址都是
  HTTPS、每个校验和都是小写 SHA-256（`backend/tests/repository_pins.rs`）。
- 插件包里的第三方许可证由组装脚本从上游包复制，`scripts/verify-package.sh` 检查它们存在；仓库根目录的
  `NOTICE` 列出全部组件和许可。

> English: `third_party/` pins every upstream input of the plugin package, and the JDT LS release the tests run as
> the user's installation, by version, HTTPS URL and SHA-256.
> The prepare scripts download into the ignored `.artifacts/` directory and verify the checksum before use; no
> upstream binaries are committed. `backend/tests/repository_pins.rs` checks the pins.
