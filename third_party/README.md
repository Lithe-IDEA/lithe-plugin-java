# 第三方输入

`third_party/` 只记录插件包需要的上游输入，每个组件一个 `manifest.json`：上游版本、HTTPS 下载地址和 SHA-256。

| 清单 | 内容 | 使用者 |
| --- | --- | --- |
| `jdtls/manifest.json` | Eclipse JDT Language Server、Lombok、java-debug、vscode-java-test 的版本、下载地址和 SHA-256 | `scripts/prepare-jdtls.sh` |
| `jdk/manifest.json` | 每个平台一份 Temurin JDK 21 的下载地址、SHA-256 和解压后的根目录名 | `scripts/prepare-jdk.sh` |

规则：

- 下载物放在被 `.gitignore` 忽略的 `.artifacts/` 下，先校验 SHA-256 再使用；缓存校验失败时删除重下。
  不要把 jar、JDK 或其他发布二进制提交到仓库。
- 升级组件时同时改版本、地址和 SHA-256，并在 PR 里写明上游发布页。`cargo test` 会检查每个地址都是
  HTTPS、每个校验和都是小写 SHA-256（`backend/tests/repository_pins.rs`）。
- 插件包里的第三方许可证由组装脚本从上游包复制，`scripts/verify-package.sh` 检查它们存在；仓库根目录的
  `NOTICE` 列出全部组件和许可。
- `jdk/manifest.json` 里的 Windows 条目留给 Windows 版插件包使用，目前的打包脚本只读取 macOS 条目。

> English: `third_party/` pins every upstream input of the plugin package by version, HTTPS URL and SHA-256.
> The prepare scripts download into the ignored `.artifacts/` directory and verify the checksum before use; no
> upstream binaries are committed. `backend/tests/repository_pins.rs` checks the pins.
