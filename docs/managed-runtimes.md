# Java 语言服务运行环境

本次实现把 Java 支持的启动依赖作为两项独立运行环境管理：完整 JDK 25 负责运行服务，JDT LS 1.61.0 负责语言能力。安装先检查本机和已准备的缓存，只下载缺失部分；插件更新继续复用相同 pin 的运行环境。项目的 `java` 工具保持独立。

## 固定输入与来源

| 依赖 | 本次固定版本 | 来源与许可 |
| --- | --- | --- |
| 完整 JDK | Microsoft Build of OpenJDK 25.0.5.0.2，build +1 | [官方下载页](https://learn.microsoft.com/en-us/java/openjdk/download)、[许可说明](https://learn.microsoft.com/en-us/java/openjdk/faq)、[对应源码](https://github.com/microsoft/openjdk-jdk25u/tree/release/jdk-25.0.5.0.2_1)；GPLv2 + Classpath Exception，保留各组件 notices |
| JDT LS | 1.61.0，202609031315 | [官方归档目录](https://download.eclipse.org/jdtls/milestones/1.61.0/)、[上游项目](https://github.com/eclipse-jdtls/eclipse.jdt.ls)；EPL 2.0，保留各组件许可 |
| 随包扩展 | Lombok、java-debug、java-test、插件自己的 project-model bundle | 继续使用 `third_party/jdtls/manifest.json` 中的固定版本和校验和 |

JDK 六个平台的不可变 Microsoft 下载地址与完整 SHA-256 都写在 `plugin.json`。它们由官方版本下载链接及 `.sha256sum.txt` 核实，不使用 latest URL。六个归档的顶层目录均为 `jdk-25.0.5.0.2+1`；macOS 的 home 是其下 `Contents/Home`，Linux / Windows 是顶层本身。Windows 使用 ZIP 和 `bin/java.exe`；另外两个系统使用 tar.gz 和 `bin/java`。

JDT LS 归档 SHA-256 为 `338e7e73d61836651ba2453919a0d34fa763eb4e7c03342092309bffb8934c64`。它同时是构建扩展、运行环境安装和文件指纹的唯一版本输入。1.61.0 核心 bundle 的上游最低运行要求是 Java 21；本插件选择 Java 25 作为完整配套环境，不把最低要求误写成 25，也不因此把项目 Java 版本提高。

## 选择与准备

在“语言与框架 → Java”查看上次启动检查实际选用的服务和 JDK 路径。首次启动前显示尚无结果。点击“检查本机语言服务器目录”可查看所选目录，或查看自动候选的来源、路径及目录检查结果。目录检查读取文件名和大小。启动与工具链校验仍核对内容指纹。

macOS 还会在用户的 `Library/Application Support` 下最多读取 256 个直属条目，只检查每项的 `tools/jdtls`。若安装位置超出范围，可直接选择目录。语言页上的服务 JDK 行可打开工具链页，在那里准备或校验缺少的组件。项目编译用的 JDK 单独设置。

`java.resolveRuntimes` 是包内 Rust 后端提供的只读函数。它不联网、不下载、不执行候选程序、不修改安装目录；宿主负责官方归档的下载、SHA-256 校验、安全解包、文件清单、并发锁和完成记录。

解析顺序是显式用户选择、已有受管候选、自动发现。服务 JDK 的显式选择来自宿主工具链事实；JDT LS 的显式选择来自 `languageServerHome`。显式配置无效时给出中英恢复提示，不悄悄覆盖用户选择。普通缺失会返回 `missing` 列表，是正常安装计划。

JDK 必须是稳定的完整 Java 25，且 release 文件的 OS/CPU 与 `java`、`javac` 的 ELF / Mach-O / PE 头一致；两个启动文件必须为非空的本机可执行文件，并具备 `lib/modules` 与 `jmods/java.base.jmod`。JRE、其他 Java 大版本、EA、错架构和明显残缺的安装会被拒绝。宿主提供的工具安装事实已经经过有界版本探测；插件不会执行可能来自不受信任目录的发现候选。

JDT LS 不按“目录里有 launcher”或“版本够新”放行。`third_party/jdtls/runtime-files.json` 固定所有 114 个 `plugins/` 文件与五个原始配置模板；解析要求完整插件文件集合一致、拒绝额外混装 bundle，并逐文件比对大小及 SHA-256。选择当前平台模板后再比较其摘要。其他编辑器安装中只有与这个验证基线一致的服务器可被复用。可写 Equinox 运行配置不会参与原始模板的替换。

| 已通过校验的本机依赖 | `missing` | 宿主准备 |
| --- | --- | --- |
| 两者都有 | 空 | 无下载 |
| 只有 JDK | `jdtls` | 只准备 JDT LS |
| 只有 JDT LS | `jdk` | 只准备完整 JDK |
| 两者都没有 | `jdk`、`jdtls` | 分别准备两个依赖 |

外部 JDK 的检查是发行版、版本、架构和完整布局资格检查，并非对任意厂商 JDK 的逐文件白名单。已管理的安装还接受宿主的归档摘要和文件清单校验。安装/更新前进行完整文件哈希核验；启动使用清单元数据检查与插件的兼容性校验。实际启动、项目导入和扩展运行仍可能暴露上游或外部安装的问题。

## 缓存和项目隔离

宿主把运行环境放在该插件的独立 `state/runtimes`。Core 的 cache key 绑定依赖声明、版本、平台和归档身份，不包含插件版本；每项依赖使用私有临时解包目录，完成验证后发布 `install-*` 世代和 `ready.json`。失败或取消不会把半成品当作就绪环境。校验过的归档可用于离线修复；坏缓存仅替换受影响依赖，已运行的旧世代保持可读。

插件版本更新但依赖不变时，不重复下载。只升级一项依赖时，另一项复用。卸载插件会随插件状态清理运行环境；旧世代目前不自动 prune，以免打断仍在运行的进程或回滚。

解析结果包含完整 `runtimes` / `missing` 分区。准备完成后，宿主把它转为 `java.resolveLaunch` 的映射：

```json
{
  "runtimes": {
    "jdk": {"home": "/approved/jdk-home", "version": "25.0.5.0.2"},
    "jdtls": {"home": "/approved/server-home", "version": "1.61.0"}
  }
}
```

启动使用批准的 JDT LS 目录并再次检查指纹；JDK 只绑定 `jdtls-runtime`，不改 `java`、全局 `JAVA_HOME`、项目的 source/target/release、Maven 或 Gradle 设置。项目仍通过原有 `java` 工具和 JDT runtime 配置选择 JDK。未显式选择项目 JDK 时，JDT 的默认行为可能使用服务 JDK；旧项目应选择其实际项目 JDK。

JDT LS、JDK 和已安装插件在运行期只读。Equinox 配置副本、工作区索引、日志和临时文件写入宿主传入的用户状态目录。缓存齐全后的语言服务启动无需下载，但项目依赖仍可能由 Maven / Gradle 联网解析。

## 权限、诊断和版本门槛

清单增加 `requires.managedRuntimes: 0` 和 `runtimes.install`，由支持此能力的配套 Lithe / Core 执行。后端继续使用已有固定 Rust SDK，因为该 hook 采用现有函数 JSON 协议。独立的 `contract.lock.json` 固定支持该能力的 Core `7f1fc4becd0b4b415aa463abd0ea5e2810b3e964` 及 manifest/native-ui/l10n schema 摘要；已有 CI / Release 调用的 `verify-sdk-lock.sh --download` 会在隔离 Python 环境中用固定 jsonschema 4.26.0 验证，不依赖本地 Core 检出。旧宿主不具备新声明的实现，不能把新包当作已兼容发布。

正常缺失和成功读取不记录 ERROR / INFO。SDK 为失败记录稳定类别 `runtime.invalid_params`、`runtime.server_incompatible`、`runtime.jdk_invalid`、`runtime.not_prepared`、`runtime.profile_invalid`，不输出路径或工具输出；取消保持取消语义。宿主提供 `language` 时，`zh` 前缀采用中文，其他语言回退英文。

## 验证入口与边界

单元测试覆盖四种存在组合、项目工具不变、显式选择、缓存优先、错误 JDK 大版本/架构、JRE、空文件、非可执行文件、JDT 缺失/损坏/额外 bundle、取消、中英错误以及六个平台的文件布局与原生文件头。清单测试检查全部 pin 与平台覆盖，profile 生成器只在审查过的上游升级时重写。

Linux x64 的实际输入为上述 Microsoft 完整 JDK 和 JDT LS。测试包调用真实 SDK 后端，连续两次解析已准备环境，再通过批准的 map 启动真实 JDT LS；initialize/shutdown 后插件、JDT LS、JDK 的完整内容和权限快照不变。一次观测的完整 JDT 文件校验为 62.1 / 58.1 ms，属于该机器的热文件缓存观测，不是跨平台性能保证。

```sh
cargo test --locked --workspace
node scripts/package/jdtls-runtime-profile.mjs .artifacts/jdtls
node scripts/package/build-linux.mjs x64
node scripts/package/smoke-linux.mjs dist/Lithe-Java-Support-4.0.2-linux-x64.zip
```

可用 `JAVA_HOME` 或 `LITHE_TEST_JDTLS_JAVA` 指向完整 Java 25。没有显式服务 Java、且 runner 的默认 JDK 不是 25 时，测试 helper 按同一官方 pin 在 `.artifacts` 准备完整 JDK；原有 CI 编译扩展的 JDK 21 和 workflow 均不需要修改。`scripts/compiler/test-live.mjs <extracted-package>` 使用同一 hook + map 流程，验证实际 JDT 导入、外部 JAR、Java 8 字节码和运行结果；项目运行 JDK 可用 `LITHE_TEST_PROJECT_JAVA_HOME` 独立指定。

`test-host-managed.mjs` 把 Java 仓库持有的 Go fixture 临时 overlay 到配套 host，调用真实 Core、SDK 后端、`pluginservice.EnsureRuntimes`、ArchiveDownloader 和 Manager。它只替换 HTTP transport，用已从官方核验下载的完整归档作为字节流，验证实际解包、启动/插件更新零新增传输、服务工具隔离、JDK 损坏后的离线修复。测试不修改 host 源码，不把 Java 特例放入宿主，也不把网络耗时当可靠性门槛。它要求没有可复用外部环境的隔离 Linux x64 runner。本轮真实宿主测试通过，测试本体 35.66 秒；两个输入各传输一次，后续启动、插件更新和 JDK 修复均没有额外传输。

```sh
LITHE_CORE_RUST_DIR=/path/to/paired-core \
node scripts/package/test-host-managed.mjs /path/to/paired-host \
  dist/Lithe-Java-Support-4.0.2-linux-x64.zip
```

默认输入位于 `.artifacts/managed-jdk/microsoft-jdk-25.0.5.0.2-linux-x64.tar.gz` 和 `prepare-jdtls.sh` 的固定归档缓存；可用 `LITHE_MANAGED_TEST_JDK_ARCHIVE` / `LITHE_MANAGED_TEST_JDTLS_ARCHIVE` 覆盖，仍须通过清单摘要核验。

macOS、Linux ARM64、Windows x64 和 Windows ARM64 的真实运行仍待各自 runner 验证。官方下载存在、归档结构可读或跨平台文件头单测不能替代原生测试；Windows ARM64 依赖声明也不代表宿主/插件已经发布 ARM64 二进制。配置错误、旧项目模型、私有仓库认证、JSP 支持与任意 Maven/Gradle 版本兼容性不由运行环境自动准备解决。
