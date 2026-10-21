# mihomo-tui Linux 服务器使用手册

本文档说明如何在 Linux systemd 服务器上安装和使用 `mihomo-tui`。当前发布物包含
`x86_64` / `aarch64` 的 Deb、RPM 和原生 ELF 二进制。

## 1. 运行模式

`mihomo-tui` 有两种互斥模式：

- 本机托管模式：不带路径或 Controller 参数运行 `sudo mihomo-tui`。程序管理
  `/etc/mihomo-tui/config.yaml`、本机 `mihomo.service` 和托管内核。
- 外部 Controller 模式：使用 `--controller` 连接已有 Mihomo。该模式不会安装内核、
  修改 systemd 或重载本机服务。

查看内置说明：

```bash
mihomo-tui -h
mihomo-tui core -h
mihomo-tui core status -h
mihomo-tui core upgrade -h
```

## 2. 选择并安装发布物

Deb 适用于 Debian/Ubuntu，RPM 适用于 Fedora/RHEL 系发行版。这两种包都包含经过固定和
校验的 Mihomo 内核、systemd unit 与本文档。先在下载目录校验对应 SHA-256，再安装：

```bash
sha256sum --check mihomo-tui_*.deb.sha256
sudo apt install ./mihomo-tui_*.deb
```

```bash
sha256sum --check mihomo-tui-*.rpm.sha256
sudo dnf install ./mihomo-tui-*.rpm
```

原生 ELF 只包含 TUI，不包含 Mihomo 内核和 systemd unit，主要用于连接已运行的外部
Controller：

```bash
sha256sum --check mihomo-tui-*-linux-$(uname -m).sha256
sudo install -m 755 mihomo-tui-*-linux-$(uname -m) /usr/local/bin/mihomo-tui
MIHOMO_SECRET='your-secret' mihomo-tui --controller http://127.0.0.1:9093
```

安装后检查程序和托管内核清单：

```bash
mihomo-tui --version
mihomo-tui core status
```

Deb/RPM 不会自动启动或启用 `mihomo.service`，也不会覆盖已有的 Mihomo 安装。如果安装器
报告路径冲突，应先确认旧安装的来源和配置，不要直接删除仍在使用的文件。

## 3. 首次启动

建议保留第二个 SSH 会话，以便配置或网络发生问题时查看日志。启动本机管理界面：

```bash
sudo mihomo-tui
```

第一次运行时：

1. 如果 `/etc/mihomo-tui/config.yaml` 已存在，程序直接读取它。
2. 否则如果 `/etc/mihomo/config.yaml` 存在，程序完整导入该配置，原文件保持不变。
3. 两者都不存在时，程序创建一个最小原生 Mihomo 配置。

常用按键：

- `1` 到 `4`：状态、代理、规则和配置页面。
- 配置页 `a`：添加 HTTP 订阅；`e`：修改所选订阅地址；`r`：更新订阅。
- 状态页 `t` / `d`：编辑 TUN / DNS 设置。
- 规则页 `a` / `A`：在顶部 / 底部添加规则；`e`：编辑；`s`：保存规则。
- `p`：校验当前配置并启动或重启本机 Mihomo。
- `q`、`Esc` 或 `Ctrl-C`：退出。

保存订阅、规则、TUN 或 DNS 只会修改配置文件。必须按 `p`，运行中的 Mihomo 才会加载
新配置。

## 4. 应用配置并启用开机启动

在 TUI 中按 `p` 后，程序会检查受支持的内核版本、用 Mihomo 校验配置、安装受管理的
systemd drop-in，然后执行 `systemctl restart mihomo.service`，以确保更新后的启动参数和配置
都生效。

即使配置未修改，再按 `p` 也会重新校验并重启，可用于恢复丢失的 TUN 路由。正在应用时
重复按键不会启动并发重启。此操作会短暂中断现有代理连接。

只有 TUI 明确报告应用成功后，才执行：

```bash
systemctl is-active mihomo.service
sudo systemctl enable mihomo.service
systemctl is-enabled mihomo.service
```

当前版本不会自动执行 `systemctl enable`。如果服务尚未运行，可以使用：

```bash
sudo systemctl enable --now mihomo.service
```

随后检查服务状态和最近日志：

```bash
systemctl status --no-pager mihomo.service
journalctl -u mihomo.service -n 100 --no-pager
```

## 5. 日常使用

本机配置默认只有 root 可读写，因此本机管理模式使用：

```bash
sudo mihomo-tui
```

只读查看内核清单不需要 root：

```bash
mihomo-tui core status
```

### 连接与实时日志

TUI 的 `5` 连接页每秒获取一次活跃连接快照。可以按 `/` 搜索目标、入口、规则或链路，
按 `Enter` 查看详情。列表不是历史记录；刷新失败时保留旧快照并明确提示。最多显示
512 条，超限会提示截断，不能据此断言整机没有其他连接。`r` 手动刷新。

`6` 日志页通过 Controller 获取 info 及以上实时日志，不改核心日志级别。`Space` 暂停
展示但继续收集，`j` / `k` 或方向键滚动，`PageUp` / `PageDown` 翻页，`End` 回到最新，
`c` 清空本地缓冲，`r` 重连。仅保留最近 500 条；单行输入上限 16 KiB，展示文本上限
2048 字符，传输队列上限 128 条，队列丢弃计数可见。

离开日志页会取消订阅，当前 HTTP 等待最长约 3 秒结束；整个 TUI 只维护一个日志工作线程。
日志采用有超时的 HTTP 流重连，重连期间可能遗漏，连续 3 次空闲/失败后暂停自动重连，
按 `r` 或重新进入页面重试。空闲没有日志不代表核心故障。暂停展示最多另保留 500 条冻结快照。
URL、已知 API 密钥与终端控制字符会被处理，但节点名、域名/IP 等仍可能敏感，不能直接公开。

需要完整历史或跟踪服务日志时仍使用：

```bash
journalctl -u mihomo.service -f
```

### 只读诊断与真实探测

```bash
sudo mihomo-tui doctor
sudo mihomo-tui doctor --json
sudo mihomo-tui doctor --probe
```

默认只读检查配置、Controller、服务、TUN 设备与 IPv4 策略路由，不向互联网目标发起探测，
不会初始化缺失配置、安装/启动/重启核心或修改 networkd。读取本机私有配置通常需要 root；
不具备权限的检查显示未知。可用 `--workspace`（优先）或 `--config` 在 `doctor` 前指定
只读配置来源；诊断拒绝符号链接、设备与管道。

报告状态区分 `pass`、`warning`、`fail`、`unknown`、`not_applicable`。JSON 的
`schema_version` 当前为 `1`，包含采集 Unix 时间戳、适用范围和 `checks` 列表。
发现明确的 `fail` 返回退出码 1；其他报告返回 0，**0 不等于所有网络检查都已通过**。
命令参数错误通常返回 2。不能把路由存在、网卡 UP 或 API 可用当成真实流量已进入代理。
设备名和路由表编号会用本地配置补足 API 未提供的字段；尚未应用的配置可能与运行核心
不同。报告中的结构失败应结合待应用状态理解，不要据此自动改路由或重启服务。

`--probe` 使用独立客户端请求固定的 `https://github.com/`，不读取代理环境变量，不显式
设置本地代理，不跟随重定向，也不会向目标发送 Controller 密钥或读取完整响应体。
同时观察目标相关的新连接，分别展示 HTTP 结果和入口/规则/链路。短连接可能漏采，多个
并发请求可能无法区分；这两种情况会保留未知/警告，不能误判为接管失败或成功。
IPv6、完整 DNS 路径和持续健康不在本轮验证范围内。

`7` 诊断页复用相同逻辑，进入时默认只读采样；`r` 重采，`g` 明确触发联网探测。
后台采集不阻塞页面切换与退出，同类检查不并发叠加。

外部 Controller 可这样排查：

```bash
MIHOMO_SECRET='your-secret' \
  mihomo-tui --controller http://127.0.0.1:9093 doctor --json
```

外部模式不读取本机配置或检查本机 systemd/网卡/路由，也不会在本机请求 GitHub 冒充远端
探测；要验证远端实际接管，应登录对应服务器运行本机 `doctor --probe`。外部 Controller
直连且不跟随 HTTP 重定向；请使用实际 API 地址和 HTTPS/专用网络/SSH 转发。

不要把 `/etc/mihomo-tui/config.yaml` 或其备份直接粘贴到公开问题中；文件可能包含
Controller 密钥和订阅凭据。

## 6. 连接外部 Controller

外部模式适合只查看或操作已经由其他工具管理的 Mihomo：

```bash
MIHOMO_SECRET='your-secret' \
  mihomo-tui --controller http://127.0.0.1:9093
```

不要把密钥直接写进命令行参数，因为命令行可能出现在 shell 历史和进程列表中。远程
Controller 不应通过公网明文 HTTP 暴露；优先使用 HTTPS、专用网络或 SSH 端口转发。

外部模式可以离线编辑显式指定的配置，但不会管理本机 systemd：

```bash
mihomo-tui \
  --controller http://127.0.0.1:9093 \
  --workspace /path/to/config.yaml
```

## 7. 升级应用和 Mihomo 内核

安装新 Deb/RPM 只注册新的候选内核，不会立即切换当前内核。推荐流程：

```bash
sudo apt install ./mihomo-tui_NEW_VERSION.deb
# RPM 系统使用：sudo dnf upgrade ./mihomo-tui_NEW_VERSION.rpm
mihomo-tui core status
sudo mihomo-tui core upgrade
mihomo-tui core status
systemctl status --no-pager mihomo.service
```

`core upgrade` 会校验候选内核和当前配置，原子切换版本，重启服务，并检查 Controller
的 `/version` 与 `/proxies`。健康检查失败时会自动切回旧内核并再次检查服务。

内核自动回滚不等于配置自动回滚。升级前应先确认当前配置已经成功应用。

## 8. 配置备份与配置恢复

每次 TUI 写入配置时都会在 `/etc/mihomo-tui` 中保留 mode `0600` 的备份：

```text
config.yaml.<timestamp>.mihomo-tui.bak
```

查看最近备份：

```bash
sudo ls -1t /etc/mihomo-tui/config.yaml.*.mihomo-tui.bak
```

当前版本没有内置恢复和保留数量限制。配置应用失败时，正式配置文件仍保留刚才保存的
内容，应先在 TUI 中修正；需要紧急恢复时，在第二个 SSH 会话中执行：

```bash
sudo systemctl stop mihomo.service
sudo install -o root -g root -m 600 \
  /etc/mihomo-tui/config.yaml.TIMESTAMP.mihomo-tui.bak \
  /etc/mihomo-tui/.config.restore.yaml
sudo mv /etc/mihomo-tui/.config.restore.yaml /etc/mihomo-tui/config.yaml
sudo /usr/lib/mihomo-tui/current/mihomo -t -d /etc/mihomo-tui
sudo systemctl restart mihomo.service
```

将 `TIMESTAMP` 替换为实际备份名。只有配置校验命令成功后才重启服务。定期把确认不再
需要的旧备份转移到受保护的归档位置，避免凭据和备份无限增长。

## 9. 常见问题

### 提示需要 root

本机安装、配置应用、服务管理和内核升级必须使用 `sudo`。外部 Controller 模式通常不
需要 root。

### 按 `p` 后配置校验失败

服务不会加载被 Mihomo 拒绝的配置，但保存的文件会保留以便修改。查看 TUI 错误和：

```bash
journalctl -u mihomo.service -n 200 --no-pager
```

无法修正时按“配置备份与配置恢复”恢复最近可用版本。

### 重启服务器后 Mihomo 没有运行

检查服务是否已启用：

```bash
systemctl is-enabled mihomo.service
sudo systemctl enable --now mihomo.service
```

### TUN 无法工作

确认服务器存在 `/dev/net/tun`，内核和虚拟化平台允许 TUN，并检查主机防火墙、转发和
现有路由。修改远程服务器默认路由前务必保留第二个 SSH 会话。

界面的“TUN 配置开启”来自 YAML，并没有检测网卡、策略路由或真实连接。“上次应用成功”
也不代表系统运行状态始终健康。网卡存在、API 可连接、节点测速正常，都不能单独证明
普通请求正在被 TUN 接管。

例如 GitHub 不指定代理很慢，但显式指定代理很快时，可以对照检查（端口按实际配置替换）：

```bash
curl --noproxy '*' -I --connect-timeout 5 --max-time 15 https://github.com
curl --noproxy '' --proxy http://127.0.0.1:7890 -I --connect-timeout 5 --max-time 15 https://github.com
ip -4 rule show
ip -4 route show table 2022
# 将 GITHUB_IP 替换为本次请求解析出的 IPv4 地址
ip -4 route get GITHUB_IP
```

`2022` 是 Mihomo 默认 TUN 路由表；如果配置了 `tun.iproute2-table-index`，应查询对应的表。
即使表中有指向 TUN 的路由，也需要有效的策略规则才能让流量进入该表。缺少查表入口或出现
`goto ... [unresolved]` 时，可按 `p` 或执行 `sudo systemctl restart mihomo.service` 重建，随后
复测路由和不指定代理的请求。在 Controller 的连接记录中确认入口为 `Tun`，并检查命中的
规则和代理链；仅检查 `ip route get` 不能证明最终走了代理节点而非 `DIRECT`。

#### systemd-networkd 重启后 TUN 路由失效

某些 systemd 版本默认使用 `ManageForeignRoutingPolicyRules=yes`，会清理不在 `.network`
文件中声明的外部策略规则，包括 Mihomo 创建的规则。检查本机手册、有效配置和日志：

```bash
man networkd.conf
systemd-analyze cat-config systemd/networkd.conf
journalctl -u systemd-networkd.service -n 100 --no-pager
```

确认冲突后，可由管理员添加 `/etc/systemd/networkd.conf.d/90-preserve-mihomo-policy-rules.conf`：

```ini
[Network]
ManageForeignRoutingPolicyRules=no
```

这会让 networkd 保留其他程序创建的策略规则，是全机设置，不会由 TUI 隐式写入。应评估现有
VPN、Docker/Kubernetes 等网络管理程序，并准备备用 SSH 会话和自动回滚后再使设置生效。
通常需要重启 networkd，再重启 Mihomo；操作后必须确认 SSH、容器网络和目标请求均正常。
可参考 [systemd networkd.conf 手册](https://www.freedesktop.org/software/systemd/man/networkd.conf.html#ManageForeignRoutingPolicyRules=)
及 [Mihomo TUN 文档](https://wiki.metacubex.one/config/inbound/tun/)。

### Controller 无法连接

检查配置中的 `external-controller`、`secret`，以及服务日志。默认推荐监听
`127.0.0.1`；不要为方便调试而把无保护的 Controller 暴露到公网。

## 10. 卸载

```bash
sudo apt remove mihomo-tui
# RPM 系统使用：sudo dnf remove mihomo-tui
```

卸载会停止并禁用服务，删除程序、unit 和托管内核。运行时创建的
`/etc/mihomo-tui/config.yaml` 及备份会保留，因为其中可能有仍需恢复的配置。确认归档
完成且凭据不再需要后，再由管理员单独处理该目录。运行时创建的
`/etc/systemd/system/mihomo.service.d/10-mihomo-tui.conf` 也不属于 Deb；重新安装前应由
管理员确认不再需要并处理，否则安装器会把它视为未受管理的冲突路径。

## 11. 收集诊断信息

报告问题时优先提供以下不包含完整配置的输出：

```bash
mihomo-tui --version
mihomo-tui core status
sudo mihomo-tui doctor --json
systemctl is-active mihomo.service
systemctl is-enabled mihomo.service
systemctl status --no-pager mihomo.service
journalctl -u mihomo.service -n 200 --no-pager
sudo stat -c '%U:%G %a %n' /etc/mihomo-tui /etc/mihomo-tui/config.yaml
```

发送前仍应检查日志中是否包含节点名、域名或其他敏感信息。
