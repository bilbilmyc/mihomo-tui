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
- `p`：校验当前配置并启动或重载本机 Mihomo。
- `q`、`Esc` 或 `Ctrl-C`：退出。

保存订阅、规则、TUN 或 DNS 只会修改配置文件。必须按 `p`，运行中的 Mihomo 才会加载
新配置。

## 4. 应用配置并启用开机启动

在 TUI 中按 `p` 后，程序会检查受支持的内核版本、用 Mihomo 校验配置、安装受管理的
systemd drop-in，然后执行 `systemctl reload-or-restart mihomo.service`。

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

跟踪服务日志：

```bash
journalctl -u mihomo.service -f
```

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
systemctl is-active mihomo.service
systemctl is-enabled mihomo.service
systemctl status --no-pager mihomo.service
journalctl -u mihomo.service -n 200 --no-pager
sudo stat -c '%U:%G %a %n' /etc/mihomo-tui /etc/mihomo-tui/config.yaml
```

发送前仍应检查日志中是否包含节点名、域名或其他敏感信息。
