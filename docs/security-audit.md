# 依赖审计记录

## 2026-10-22：修复 GitHub CI 安全审计失败

最新应用代码能够构建，但锁文件仍固定在较旧的 TLS/证书与可选 QUIC 依赖。对修复前
Cargo.lock 运行 `cargo audit 0.22.2`，复现了 GitHub 报告的 11 条漏洞记录，退出码为 1。

本次只定向更新四个传递依赖及对应 checksum，不修改 Cargo.toml 的功能选择，不升级
Mihomo 核心，也不通过忽略告警、过滤目标平台或关闭审计来绕过检查：

| 依赖 | 原版本 | 修复后版本 | 原报告 |
| --- | --- | --- | --- |
| quinn-proto | 0.11.16 | 0.11.19 | RUSTSEC-2026-0338、0339、0340、0341、0342 |
| rustls | 0.23.42 | 0.23.46 | RUSTSEC-2026-0285、0346、0348、0349 |
| rustls-webpki | 0.103.13 | 0.103.16 | RUSTSEC-2026-0382 |
| webpki-roots | 1.0.9 | 1.0.10 | RUSTSEC-2026-0381 |

quinn-proto 虽不在当前 Linux 功能选择的实际构建树中，仍保留在锁文件中，因此同样更新；
不能因为当前未启用而把整份锁文件的告警隐藏。更新后同一审计命令报告 **0 条漏洞记录**，
退出码为 0。这只是针对当时 advisory 数据库的结果，不代表所有依赖没有风险。

## 仍然可见的警告

当前审计还有未维护、健全性和撤回版本警告，未加入忽略列表，也没有声称已经解决：

- `paste 1.0.15`（RUSTSEC-2024-0436）：上游不再维护，由 Ratatui 引入。
- `unsafe-libyaml 0.2.11`（RUSTSEC-2024-0449）：上游不再维护，由 serde_yaml 引入。
- `lru 0.12.5`（RUSTSEC-2026-0002、0253）：健全性警告，由 Ratatui 0.29 引入。
  修复需要评估上游 UI 依赖升级及行为回归，而不是强行把不兼容版本替换进锁文件。
- `chacha20 0.10.1`：撤回版本警告，存在于锁文件的可选依赖链中。

这些不应通过 `--ignore` 消失。UI/YAML 依赖迁移须单独验证，不能为让日志全绿而随意改写
有凭据的配置序列化、终端行为或上游依赖契约。

## 复查命令

```bash
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --file Cargo.lock
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

CI 的 RustSec 检查保持原有阻断行为，产物上传在质量与安全检查通过后进行。该记录不替代
持续审计；日后新增 advisory 可能再次阻断构建，需要重新处理。
