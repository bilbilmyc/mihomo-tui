# Linux 发布物

## 产物约定

CI 在 x86_64 和 aarch64 上执行原生构建。每个架构任务生成三种发布物，并为每个发布物生成一个 SHA-256 文件：

| 格式 | x86_64 文件名 | aarch64 文件名 | 内容 |
| --- | --- | --- | --- |
| Deb | `mihomo-tui_<version>+mihomo<core>-1_amd64.deb` | `..._arm64.deb` | TUI、固定内核、unit、文档 |
| RPM | `mihomo-tui-<version>-1.mihomo<core>.x86_64.rpm` | `...aarch64.rpm` | TUI、固定内核、unit、文档 |
| 原生二进制 | `mihomo-tui-<version>-linux-x86_64` | `...-linux-aarch64` | 仅 TUI ELF |

Deb 和 RPM 使用相同的 `/usr/lib/mihomo-tui` 托管内核布局，并在替换软件包时保留当前选中的内核。安装过程不会启用或启动 `mihomo.service`。全新安装时，两种包都会拒绝已有且未受管的 Mihomo 二进制、unit 或托管内核目录。

原生发布物有意只包含 Rust 可执行文件，可在兼容 glibc 的 Linux 上连接已有 Controller。由于运行时包校验目前只接受官方 Deb 元数据，本机托管自动安装仍限于 Debian/Ubuntu。

## 本地构建

只能在目标架构的原生主机上构建；打包器会拒绝跨架构二进制：

```bash
sudo apt-get install cpio curl dpkg-dev file jq rpm
cargo build --release --locked

rust_arch=$(uname -m)
deb_arch=$(dpkg --print-architecture)
./scripts/build-native.sh --architecture "$rust_arch" --binary target/release/mihomo-tui
./scripts/build-deb.sh --architecture "$deb_arch" --binary target/release/mihomo-tui
./scripts/build-rpm.sh --architecture "$rust_arch" --binary target/release/mihomo-tui
```

不安装软件包即可执行以下验证：

```bash
./scripts/test-native-binary.sh dist/mihomo-tui-*-linux-"$(uname -m)"
./scripts/test-deb-package.sh dist/*.deb
./scripts/test-rpm-package.sh dist/*.rpm
```

一次性主机测试会破坏该主机上已有的 `mihomo-tui` 软件包。CI 在全新的 GitHub runner 上测试 Deb，并在全新的 Fedora 容器中测试 RPM：

```bash
sudo ./scripts/test-install-deb.sh dist/*.deb
docker run --rm \
  --volume "$PWD:/work:ro" \
  --workdir /work \
  fedora:42@sha256:99e203b80b1c3d8f7e161ec10a68fd02b081ef83a3963553e513c82846b97814 \
  bash -lc 'dnf install -y jq && ./scripts/test-install-rpm.sh dist/*.rpm'
```

## CI 与发布

每个 Pull Request 和分支 push 都会运行格式检查、Rust 测试、Clippy、RustSec 和发布策略测试。
质量与安全门禁通过后，两个原生架构的软件包任务构建并检查全部格式，在一次性 runner 上
安装 Deb、在 Fedora 中安装 RPM，验证重新安装时保留当前版本，然后上传可下载产物。

### 下载分支构建

在仓库 **Actions → 对应提交的 CI → Artifacts** 中选择：

- `mihomo-tui-linux-x86_64-<commit-sha>`
- `mihomo-tui-linux-aarch64-<commit-sha>`

每份 ZIP 包含对应架构的 Deb、RPM、原生 ELF 和三份 SHA-256 文件，保留 **7 天**。
下载通常需要登录 GitHub；下载后先解压，再在解压目录校验：

```bash
sha256sum --check -- *.sha256
# 如果使用原生 ELF：ZIP 不保留可执行权限，需要手工设置
chmod 755 mihomo-tui-*-linux-x86_64
```

arm64 主机将上述文件名后缀替换为 `aarch64`。这是指定提交的测试构建，**不是正式 Release**；
文件内版本号仍来自 Cargo.toml，不会因普通 push 自动递增。安装前核对提交、架构和校验和。
CI 只上传已验证的公开发布物，不上传配置、私有快照或隐藏文件。上传失败或存储配额不足会
使该软件包任务失败，不会假报有下载；可在配额恢复后重跑对应任务，保留期也不能保证绕过账户配额。

### 正式标签发布

标签必须与 Cargo 版本完全一致，例如 `v0.0.1`。质量和安全门禁通过后，CI 会创建或复用一个 GitHub Release 草稿。x86_64 与 aarch64 任务随后进行原生构建和验证，并把文件直接上传到该草稿。最终任务下载全部 12 个文件并逐一校验哈希。草稿必须由维护者审查和发布；重新运行工作流会替换名称相同的草稿附件，而不是创建重复 Release。CI 会拒绝修改已经发布的 Release。

标签发布和托管内核提案继续复用不包含 artifact 上传的打包 action；正式 Release 的跨任务
文件传递仍直接通过草稿附件，不依赖临时 Actions artifact 存储。

每周运行的托管内核提案工作流使用同一个打包 action。因此，上游版本变更只有在两个架构的 Deb、RPM、原生二进制和一次性安装检查全部通过后，才能创建 Pull Request。

## 发布门禁

CI 会为版本标签生成经过验证的 Release 草稿，但公开发布还必须满足：

- 发布物携带 `mihomo-tui` 的 MIT 许可证；
- 确认 Deb `Maintainer` 与 RPM `Packager` 使用可联系的项目维护者地址；
- 审查固定的 Mihomo 版本、哈希、许可证文本与源码声明；
- x86_64 和 aarch64 软件包任务均成功；
- 维护者人工审查 Release 草稿。

`mihomo-tui` 采用 MIT License；随包 Mihomo 继续适用 GPL-3.0，并携带独立的许可证和源码声明。
