# Debian 软件包运维

## 适用范围

`mihomo-tui` Deb 是面向原生 amd64 和 arm64 Debian/Ubuntu systemd 主机的独立发行包。它同时提供一个 Rust TUI 和一个经过审查的官方 Mihomo 内核，但 Mihomo 仍作为独立进程运行。

该软件包与上游 `mihomo` 软件包冲突，因为两者都会提供 `mihomo.service`。当 Mihomo 由本软件包之外的方式管理时，仍可使用外部 Controller 模式。

## 构建与验证

安装原生 Rust 工具链以及 `scripts/build-deb.sh` 使用的 Debian 构建工具，包括 `curl`、`jq`、`file`、`dpkg-dev` 和 systemd 工具。只能在目标架构上构建：

```bash
./scripts/build-deb.sh --architecture "$(dpkg --print-architecture)"
./scripts/test-deb-package.sh dist/*.deb
```

构建器默认始终使用 `--locked` 重新编译 `target/release/mihomo-tui`。也可以通过 `--binary PATH` 指定另行控制的原生构建，但构建器仍会检查其版本、可执行文件类型、符号链接状态和架构。

构建器会在信任边界执行以下检查：

- 只接受 `managed-core.json` 中规范的 Release、产物、架构、Deb 版本、哈希和 GPL-3.0 许可证元数据；
- 将内核 Deb 大小限制为 128 MiB，将许可证文件限制为 1 MiB；
- 解包前校验软件包 SHA-256、`Package`、`Version` 和 `Architecture`；
- 只提取并执行精确路径下的普通文件 `usr/bin/mihomo`；
- 要求内核报告的版本与清单推荐版本完全相同；
- 使用 `dpkg-shlibdeps` 从实际 Rust ELF 推导 `libc6` 和 `libgcc-s1` 依赖；
- 通过 `dpkg-deb --build --root-owner-group` 构建归档，并写出权限为 `0644` 的校验文件。

`scripts/test-deb-package.sh` 不安装软件包，而是解包并检查元数据、载荷路径、权限、unit 语法、原生可执行文件版本、许可证哈希、源码声明和校验和。

## 安装布局

```text
/usr/bin/mihomo-tui
/usr/lib/mihomo-tui/bundled/<version>/mihomo   软件包拥有的载荷
/usr/lib/mihomo-tui/cores/<version>/mihomo     受管的不可变硬链接
/usr/lib/mihomo-tui/current                    当前版本的相对符号链接
/usr/lib/systemd/system/mihomo.service
/usr/share/doc/mihomo-tui/Mihomo-LICENSE
/usr/share/doc/mihomo-tui/Mihomo-NOTICE
/usr/share/doc/mihomo-tui/server-guide.md
/etc/mihomo-tui/config.yaml                    运行时创建的原生配置
```

`postinst` 会先校验 root 所有权和父目录不可写权限，再把载荷硬链接到 `cores`。如果某版本已经存在但字节内容不同，它绝不会覆盖。硬链接既避免复制第二份内核，也可在 dpkg 删除旧软件包载荷时保留受管 inode。

软件包不拥有 `current` 或 `cores/<version>/mihomo`。这是有意设计：安装新包只注册候选版本，不会删除当前回滚内核，也不会切换激活版本。

## 全新安装

使用 APT 安装，以便解析声明的运行时依赖：

```bash
sudo apt install ./dist/mihomo-tui_*.deb
mihomo-tui core status
sudo mihomo-tui
```

systemd 运行时，软件包安装会执行 `systemctl daemon-reload`，但不会启用或启动 `mihomo.service`。在干净主机上，由于不存在 `current` 链接，`postinst` 会选择随包版本。运行 TUI 会初始化 `/etc/mihomo-tui/config.yaml`；按 `p` 才会显式校验配置并启动或重载服务。

全新安装解包前，`preinst` 会检查 `/etc`、`/run`、`/usr/lib` 和 `/lib` 下的常见路径，并拒绝已有的 Mihomo 或 `mihomo-tui` 二进制、托管内核根目录、Mihomo unit、drop-in 或启用链接。这样可防止软件包接管未受管的现有安装。软件包升级时这些路径本来就归本包所有，因此不会执行该防护检查。

## 软件包与内核升级

安装经过审查的新软件包只是准备候选版本，不会立即激活：

```bash
sudo apt install ./mihomo-tui_NEW_VERSION.deb
mihomo-tui core status
sudo mihomo-tui core upgrade
```

执行 `core upgrade` 前，受管配置必须是可信的 root 文件并定义 `external-controller`；已加载的 `mihomo.service` 必须来自软件包 unit；当前内核也必须仍位于受测兼容范围。

显式升级会取得共享 root 锁，优先复用软件包注册的候选版本，用该精确二进制校验配置，原子切换 `current`，重启服务，然后轮询 `/version` 与 `/proxies`。任何激活或健康检查失败都会切回旧版本、重启并检查回滚健康状态；如果回滚也失败，错误中会同时报告两个失败原因。普通启动和配置应用绝不会调用该事务。

## 卸载

```bash
sudo apt remove mihomo-tui
```

卸载会禁用并停止 `mihomo.service`，删除软件包载荷、当前链接和受管内核二进制，然后重新加载 systemd。运行时创建的 `/etc/mihomo-tui/config.yaml` 及其备份不归软件包所有，不会自动删除。只有确认配置和凭据都不再需要后，才应单独删除这些文件。

## CI 与发布

每个 Pull Request 都会执行 Rust 格式检查、测试、Clippy、RustSec、软件包策略测试以及原生 amd64/arm64 发布构建。每个 runner 都会构建 Deb、RPM 和原生 ELF，在一次性 runner 上安装 Deb、在 Fedora 容器中安装 RPM，检查托管布局发现，模拟软件包载荷替换，验证保留当前内核，最后卸载软件包。

定时上游工作流只能更新现有兼容范围内的 Release、软件包和许可证哈希。它会推送提案分支，运行相同的双架构门禁并创建 Pull Request；绝不会在用户主机上安装、扩大兼容范围、合并变更或发布 Release。

在 CI 之外发布产物前，维护者必须：

- 审查清单差异和上游 Release Notes；
- 确认两个架构任务和回滚测试都已通过；
- 校验 Deb、RPM、原生 ELF 与 `.sha256` 的文件名和哈希；
- 保留 `Mihomo-LICENSE`、`Mihomo-NOTICE` 和对应源码 URL；
- 声明 `mihomo-tui` 项目自身的许可证和版权持有人；
- 用真实联系方式替换软件包维护者占位信息；
- 要求人工批准配套 Release。

Mihomo 载荷记录为 GPL-3.0，并固定了许可证哈希。该元数据不会为 Rust `mihomo-tui` 源码授予或选择许可证。在项目维护者完成这项独立法律决策之前，仍禁止公开发布软件包。
