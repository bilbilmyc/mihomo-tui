# mihomo-tui

An SSH-friendly terminal control center for [Mihomo](https://github.com/MetaCubeX/mihomo).

The project deliberately keeps Mihomo as the proxy core and focuses on the Linux workflow around it: profiles, proxy groups, rules, TUN, DNS, and service diagnostics.

## Current MVP

- Ratatui dashboard with Status, Proxies, Rules, and Config pages
- Automatic Mihomo installation and service startup on supported Linux hosts
- Demo mode with `--no-auto-install` when no controller or local installation is available
- Mihomo `/proxies` API discovery and refresh
- Proxy-group selection through the controller API
- Reads and edits the active Mihomo YAML for rules, providers, TUN, DNS, port, and mode
- TUN settings for stack, device, routes, DNS hijacking, MTU, and excluded networks
- DNS settings for listen address, enhanced mode, Fake IP ranges, IPv6, HTTP/3, and routing rules
- Custom rule creation at the top or bottom, editing, removal marking, and priority reordering
- Candidate validation, permission-preserving writes, backups, service reload, and automatic rollback
- ASCII-compatible UI for SSH terminals without Nerd Fonts

## Run

With no explicit controller or config, the program manages the system Mihomo installation. Build it,
then run the local-management mode as root:

```bash
cargo build --release
sudo ./target/release/mihomo-tui
```

On a clean Debian/Ubuntu host it downloads the pinned official `v1.19.29` package, verifies its
SHA-256 and deb metadata, installs it through `dpkg`, creates a loopback-only Controller
configuration, and enables `mihomo.service`. Downloaded artifacts stay root-owned from creation
through installation.

Automatic installation currently supports `x86_64` and `aarch64` Debian/Ubuntu systems running
systemd. It never upgrades an existing installation and refuses to overwrite a partial installation
where only some of the binary, service, or system config exist. If a first installation is
interrupted after `dpkg`, the next run can safely resume only when the config is still the exact
official default.

Skip downloading and enter Demo mode on a clean host:

```bash
cargo run -- --no-auto-install
```

Connect to a running Mihomo controller:

```bash
cargo run -- \
  --controller http://127.0.0.1:9093 \
  --secret "$MIHOMO_SECRET"
```

Or use environment variables:

```bash
MIHOMO_CONTROLLER=http://127.0.0.1:9093 \
MIHOMO_SECRET=secret \
cargo run
```

An explicit `--controller`, `MIHOMO_CONTROLLER`, `--config`, or `MIHOMO_CONFIG` disables startup-time
local installation and service start. A config discovered under `~/.config/mihomo` is also treated as
user-managed and does not trigger system service changes. Config edits in these external modes are
saved locally but do not reload `mihomo.service` on the TUI host.

Keys:

- `1`, `2`, `3`, `4`: switch pages
- `Tab`: next page
- `j`/`k` or arrow keys: move selection
- `r`: refresh Mihomo data; on the configuration page, update the selected HTTP provider first
- `a`: add an HTTP provider from the configuration page
- `Right` or `Enter`: enter the selected proxy group's node list; on the subscription page, open a selectable proxy group
- `Left`: return to proxy groups
- `Enter`: apply the selected node when the node list is focused
- `l`: test the selected node's latency; results display as milliseconds, `timeout`, or `failed`
- `t`/`d`: open advanced TUN/DNS settings from the status page
- `a`/`A`: add a custom rule at the top/bottom of the rule list
- `e`: edit the selected rule
- In the rule editor, press `Enter` to open type/policy lists, move with arrows, and confirm with `Space` or `Enter`
- `Space` or `x`: mark/unmark a rule for removal; `s` saves rule changes
- `J`/`K`: move a rule down/up
- `s`: save rules, validate the candidate YAML, create a backup, and reload Mihomo
- `q` or `Esc`: quit

File providers have no remote subscription URL, so `r` cannot download a new source for them.
Update the configured source file or replace the provider with an HTTP provider that has a URL.

## Planned slices

1. Profile and subscription storage with manual update and rollback
2. Mihomo process lifecycle and systemd diagnostics
3. System proxy management
4. Connections, logs, traffic, and latency views
