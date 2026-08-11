# Mihomo 托管内核

## 目标

`mihomo-tui` 随包提供并管理经过测试的官方 Mihomo 内核，同时让内核继续作为独立进程运行。用户获得一套产品和一套配置流程，但本仓库不会变成 Mihomo 源码分支，也不会把 Go 内核链接进 Rust 进程。

托管内核约定有两类使用方：

- 本机托管模式负责安装、校验、配置和重载受测内核；
- 外部模式继续使用由用户管理的 Controller，绝不改变本机运行时。

发布元数据、兼容策略、许可证身份、软件包构建、显式激活、健康回滚和上游同步都使用同一份内嵌清单。TUI 普通启动绝不检查上游，也不会激活新发现的内核。

## 技术栈

- 采用 Rust 2024，使用 Clap、Ratatui、Reqwest、Serde 和 SHA-256 校验。
- 官方 Mihomo 发布二进制始终作为独立的操作系统进程运行。
- 内嵌 JSON 清单是推荐版本、受测兼容范围、目标软件包、包元数据和哈希的唯一事实来源。
- 本机生命周期管理目前仅支持 Debian/Ubuntu systemd 主机。

## 常用命令

```bash
cargo test --all-targets
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --release
sudo ./target/release/mihomo-tui
./target/release/mihomo-tui core status
sudo ./target/release/mihomo-tui core upgrade
```

## 项目结构

```text
managed-core.json       可内嵌、可审查的官方内核发布约定
src/core/               版本解析、清单校验与兼容策略
src/system.rs           可信 root 文件、命令清理与私有临时路径
src/core_package/       下载、完整性、Deb 元数据、提取与安全临时文件
src/core_manager/       托管内核门面、清单、不可变存储与测试
src/core_upgrade/       候选编排、激活、API 健康检查与回滚
src/runtime/            运行时门面、安装、systemd 生命周期与测试
src/mihomo.rs           外部 Controller API 边界
packaging/debian/       unit 与维护者脚本模板
packaging/rpm/          标准 RPM spec 模板
scripts/lib/release.sh  共享清单、下载、ELF 与校验和策略
scripts/build-*.sh      原生 ELF、Deb 与 RPM 发布构建器
docs/managed-core.md    架构、安全边界与运维说明
```

## 代码风格

在信任边界使用显式结果类型，并保持策略函数纯净，使其无需 root、网络或文件系统即可测试：

```rust
pub fn compatibility(version: CoreVersion) -> Compatibility {
    if version < minimum {
        Compatibility::TooOld
    } else if version >= maximum_exclusive {
        Compatibility::UntestedNewer
    } else {
        Compatibility::Supported
    }
}
```

发布元数据只属于清单。运行时代码不得重复保存版本字符串、包名、架构或哈希。

## 测试策略

- 小型单元测试覆盖清单结构、支持目标、版本输出解析和每个兼容边界。
- 现有运行时测试继续覆盖 URL 白名单、大小上限、SHA-256 校验、Deb 元数据、systemd 所有权检查和安装规划。
- 每次清单或运行时变更都必须通过完整 Rust 测试、格式检查、禁止警告的 Clippy 和 release 构建。
- 一次性原生 runner 会安装和卸载各架构软件包，确认服务保持禁用和停止，验证托管布局发现，并证明重新安装软件包不会替换运维人员选中的当前内核。
- 伪激活操作会覆盖重启和健康检查成功、健康检查失败后回滚、回滚健康检查，以及激活与回滚同时失败；测试不会改变测试主机。

## 安全边界

始终遵守：

- 只通过 HTTPS 使用官方 `MetaCubeX/mihomo` GitHub Release 路径；
- 限制下载大小和重定向，校验 SHA-256 与 Deb 包元数据；
- 安装或升级内核前要求用户显式操作；
- 改变当前内核前先暂存并校验候选版本；
- 新版本通过健康检查前保留旧的可用内核；
- 外部 Controller 模式不得改变本机运行时。

实施前必须先确认：

- 扩展支持的操作系统、服务管理器、架构或 Release 主机；
- 在没有集成测试证据的情况下更改受测兼容范围；
- 更改内核安装布局，或接管已有且未受管的服务；
- 让 TUI 普通启动自动执行升级。

绝不允许：

- 把 Mihomo 源码复制进本仓库，或通过进程内 FFI 暴露；
- 静默跟随上游最新 Release；
- 在完整性和包元数据检查通过前运行下载的产物；
- 覆盖不完整安装或未受管安装；
- 显示或记录 Controller 密钥及订阅凭据。

## 托管内核生命周期

### 发布约定

- 内嵌一份经过校验的托管内核清单。
- 通过该清单按操作系统和架构选择软件包。
- 解析已安装内核版本；本机托管应用前，要求该版本位于受测兼容范围。
- 普通启动和配置应用不得包含升级行为。

### 显式分阶段升级

- `mihomo-tui core status` 报告已安装、当前、推荐和兼容版本，不改变主机。
- `sudo mihomo-tui core upgrade` 是唯一升级入口。普通启动和按 `p` 都不会调用它。
- 受管二进制位于 `/usr/lib/mihomo-tui/cores/<version>/mihomo`。root 所有的 `/usr/lib/mihomo-tui/current` 符号链接选择一个不可变版本目录。
- systemd drop-in 启动 `/usr/lib/mihomo-tui/current/mihomo -d /etc/mihomo-tui`。
- 如果推荐候选版本已经安装，升级会直接复用。否则，程序下载并校验官方 Deb，只把其中的 Mihomo 二进制提取到私有暂存目录，校验精确版本，再使用该候选版本校验受管配置。
- 首次激活前，当前可信二进制会被复制到自己的版本目录，保证始终可用于回滚。
- 激活会原子替换 `current` 链接，重新加载 systemd，重启服务，再通过受管配置发现的 Controller 检查 `/version` 与 `/proxies`。
- 任何激活、重启、版本或代理健康检查失败，都会原子恢复旧链接，重启旧内核并验证回滚健康状态。如果回滚也不健康，返回的错误会同时说明两个失败原因。
- 升级成功后保留旧内核，供自动回滚取证和运维检查，并清除无关暂存文件。在推荐版本上重新运行升级是幂等的空操作。

### 发布同步

- 通过 GitHub 文档规定的 `GET /repos/MetaCubeX/mihomo/releases/latest` 端点，只检测已发布、非草稿且非预发布的最新 Mihomo Release。
- 生成清单更新分支和 Pull Request，其中包含包元数据与哈希。工作流仅获得 `contents: write` 和 `pull-requests: write` 权限。
- 运行架构构建矩阵和一次性主机集成测试。
- 发布新的配套 `mihomo-tui` Release 前必须人工批准。
- 现有兼容范围内的补丁版本可由自动化提出；范围外的 Release 默认拒绝，必须经过兼容策略变更审查。

### 随包分发

- 使用 `dpkg-deb --build --root-owner-group` 构建 Debian 包，使用标准 `rpmbuild` 构建 RPM。两种软件包都包含 `mihomo-tui`、位于 `bundled/<version>` 的受测官方内核、`mihomo.service` unit、许可证/源码声明和维护者脚本。
- 安装时，`postinst` 校验 root 所有权、权限和同版本不可变字节后，在 `cores/<version>/mihomo` 创建硬链接。只有没有当前链接时才创建 `current`。该受管硬链接有意不归包管理器所有，因此替换软件包载荷不会删除当前或回滚内核。
- 安装新版软件包只注册其内核并保留 `current`。激活仍必须显式执行 `sudo mihomo-tui core upgrade` 事务。
- 随包版本与单独安装的 `mihomo` 软件包冲突，因为二者都会拥有同一个服务；用户必须明确选择随包托管模式或外部管理模式。
- 每个 Deb、RPM 和原生 ELF 架构产物都必须发布 SHA-256 校验文件。
- 对于自行管理 Mihomo 的用户，继续提供外部模式。

## 升级事务

```text
加锁 -> 检查当前版本 -> 复用已安装候选版本，或下载/哈希/Deb 校验/提取
     -> 候选版本与配置检查 -> 暂存不可变版本
     -> 原子切换 current -> daemon-reload -> 重启 -> API 健康检查
     -> 成功：保留旧版本
     -> 失败：原子切回旧 current -> 重启 -> 回滚健康检查
```

原子切换链接之前的每条路径，除了私有暂存区和可选的新不可变版本目录外，都不产生副作用。服务绝不会仅仅为了下载或检查候选版本而停止。

## 权威资料

- [GitHub 获取最新 Release API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)
- [GitHub Actions 工作流语法与权限](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
- [GitHub CLI 创建 Pull Request](https://cli.github.com/manual/gh_pr_create)
- Debian 归档操作使用已安装的 `dpkg-deb` 接口（`--field`、`--extract` 和 `--build --root-owner-group`），并确认它可作为可信 root 可执行文件使用。

## 成功条件

满足以下条件后，实施内容才可进入 Release 审查：

- 一份清单是推荐版本、兼容范围、包名、包版本、架构和哈希的唯一来源；
- 内嵌发布元数据格式错误或不完整时默认拒绝；
- 解析已安装内核版本输出时不使用子字符串匹配；
- 本机应用接受受测版本，并以可操作的错误拒绝过旧或未经测试的新版本；
- 外部模式和全新主机安装行为保持不变；
- 升级是显式、分阶段、带健康检查、幂等且经过回滚测试的事务；
- 上游同步创建可审查的 Pull Request，且不能自动扩大兼容范围；
- 两种支持架构的软件包都能构建，包含预期文件，通过元数据检查，在一次性 runner 上安装，并发布校验和；
- 所有验证命令均通过，且不改变主机上正在运行的服务。

## 发布门禁

随包 Mihomo 的许可证文本和对应源码 URL 必须存在，一次性主机软件包测试与回滚测试必须通过，并且人工批准 Release Pull Request，之后才能发布软件包。在公开分发前，仓库还必须声明 `mihomo-tui` 项目自身的许可证和真实的软件包维护者身份；当前自动检查覆盖随包 Mihomo 的 GPL-3.0 材料，但不能替本项目选择许可证或版权持有人。

缺少任一项目的许可证材料、源码信息、回滚证据或架构覆盖都会阻止发布。
