# mihomo-tui

An SSH-friendly terminal control center for [Mihomo](https://github.com/MetaCubeX/mihomo).

The project keeps Mihomo as an optional runtime backend while owning an independent,
versioned configuration source. Subscriptions, rules, TUN, and DNS can be edited while
Mihomo is stopped, missing, or unreachable.

## Current MVP

- Ratatui dashboard with Status, Proxies, Rules, and Config pages
- Independent configuration at `/etc/mihomo-tui/config.yaml`
- One-time, lossless import from an existing Mihomo YAML configuration
- Offline subscription, rule, TUN, and DNS editing without a Mihomo binary
- Explicit runtime apply with validation, backup, reload, rollback, and subscription verification
- Optional Mihomo installation during explicit apply on supported Linux hosts
- Mihomo `/proxies` API discovery and refresh
- Proxy-group selection through the controller API
- Reads and edits the independent profile for rules, providers, TUN, DNS, port, and mode
- TUN settings for stack, device, routes, DNS hijacking, MTU, and excluded networks
- DNS settings for listen address, enhanced mode, Fake IP ranges, IPv6, HTTP/3, and routing rules
- Custom rule creation at the top or bottom, editing, removal marking, and priority reordering
- Syntax-checked atomic source writes with private backups
- Candidate validation, permission-preserving runtime writes, service reload, and automatic rollback
- ASCII-compatible UI for SSH terminals without Nerd Fonts

## Run

Build the program, then run it as root when using the default system paths:

```bash
cargo build --release
sudo ./target/release/mihomo-tui
```

Startup does not install, start, validate, or reload Mihomo. On first use, the program imports
`/etc/mihomo/config.yaml` into `/etc/mihomo-tui/config.yaml`. If no runtime config exists, it creates
a minimal independent profile. An existing independent config is never overwritten.

The independent file has a small versioned wrapper and retains the complete Mihomo document under
`profile`, including fields mihomo-tui does not understand:

```yaml
kind: mihomo-tui/v1
backend: mihomo
profile:
  mixed-port: 7890
  rules:
    - MATCH,DIRECT
```

Edits change only the independent source. The status page shows `待应用` when it differs from the
runtime target. Press `p` to explicitly validate and apply the profile to
`/etc/mihomo/config.yaml`, reload `mihomo.service`, and verify HTTP subscriptions. Validation leaves
the runtime target untouched on failure; reload failure restores the previous runtime backup.

On a clean Debian/Ubuntu host, the first explicit apply can download the pinned official `v1.19.29`
package, verify its SHA-256 and deb metadata, install it through `dpkg`, create the runtime service,
and then apply the independent profile. Downloaded artifacts stay root-owned from creation through
installation.

Automatic installation currently supports `x86_64` and `aarch64` Debian/Ubuntu systems running
systemd. It never upgrades an existing installation and refuses to overwrite a partial installation
where only some of the binary, service, or system config exist. If a first installation is
interrupted after `dpkg`, the next run can safely resume only when the config is still the exact
official default.

Prevent downloads during explicit apply:

```bash
sudo ./target/release/mihomo-tui --no-auto-install
```

Use custom paths for an isolated or unprivileged workspace:

```bash
cargo run -- \
  --workspace "$HOME/.config/mihomo-tui/config.yaml" \
  --config "$HOME/.config/mihomo/config.yaml" \
  --no-auto-install
```

`--workspace` / `MIHOMO_TUI_CONFIG` selects the independent source. `--config` /
`MIHOMO_CONFIG` selects the legacy config imported on first use and the comparison target. An
explicit config is treated as externally managed and is never written or reloaded by mihomo-tui.

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

An explicit `--controller`, `MIHOMO_CONTROLLER`, `--config`, or `MIHOMO_CONFIG` uses external mode
and disables local apply. Offline edits remain available. An explicit controller never inherits the
secret from a local profile; provide `--secret` or `MIHOMO_SECRET` for that controller.

Keys:

- `1`, `2`, `3`, `4`: switch pages
- `Tab`: next page
- `p`: explicitly validate and apply pending independent configuration
- `j`/`k` or arrow keys: move selection
- `r`: refresh Mihomo data; on the configuration page, update the selected HTTP provider first
- `a`: add an HTTP provider from the configuration page; using an existing HTTP provider name updates its URL
- `e`: replace the selected HTTP provider URL from the configuration page
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
- `s`: save rule changes to the independent configuration
- `q` or `Esc`: quit

File providers have no remote subscription URL, so `r` cannot download a new source for them.
Update the configured source file or replace the provider with an HTTP provider that has a URL.

The complete migration, storage, and apply contract is documented in
[`docs/independent-config.md`](docs/independent-config.md).
