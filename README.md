# mihomo-tui

An SSH-friendly terminal control center for [Mihomo](https://github.com/MetaCubeX/mihomo).

The project owns one native Mihomo configuration file. Subscriptions, rules, TUN, and DNS can be
edited while Mihomo is stopped, missing, or unreachable, and the managed Mihomo service reads that
same file directly.

The Debian bundle is a standalone application distribution: it contains `mihomo-tui`, a reviewed
official Mihomo binary, the service unit, license/source notices, and the versioned managed-core
layout. Mihomo remains a separate process rather than being linked into the Rust program.

## Capabilities

- Ratatui dashboard with Status, Proxies, Rules, and Config pages
- Single native configuration at `/etc/mihomo-tui/config.yaml`
- One-time, lossless import from an existing Mihomo YAML configuration
- Offline subscription, rule, TUN, and DNS editing without a Mihomo binary
- Explicit validation, core reload, and subscription verification
- Optional Mihomo installation during explicit apply on supported Linux hosts
- Read-only `mihomo-tui core status` inventory and explicit transactional `core upgrade`
- Versioned managed cores with atomic activation, API health checks, and automatic rollback
- Native amd64 and arm64 Debian bundles with SHA-256 checksums
- Mihomo `/proxies` API discovery and refresh
- Proxy-group selection through the controller API
- Reads and edits the single native YAML for rules, providers, TUN, DNS, port, and mode
- TUN settings for stack, device, routes, DNS hijacking, MTU, and excluded networks
- DNS settings for listen address, enhanced mode, Fake IP ranges, IPv6, HTTP/3, and routing rules
- Custom rule creation at the top or bottom, editing, removal marking, and priority reordering
- Syntax-checked atomic writes with private backups
- A managed systemd drop-in that points Mihomo directly at `/etc/mihomo-tui`
- ASCII-compatible UI for SSH terminals without Nerd Fonts

## Standalone Debian Install

Build and verify a native package on an amd64 or arm64 Debian/Ubuntu host:

```bash
./scripts/build-deb.sh --architecture "$(dpkg --print-architecture)"
./scripts/test-deb-package.sh dist/*.deb
```

The builder downloads only the pinned official release from `managed-core.json`, limits artifact
sizes, verifies SHA-256 and Deb metadata, builds a fresh locked Rust binary, derives shared-library
dependencies, and emits a `.deb` plus `.sha256`.

Install the bundle with dependency resolution:

```bash
sudo apt install ./dist/mihomo-tui_*.deb
mihomo-tui core status
sudo mihomo-tui
```

Installation does not enable or start `mihomo.service`. On a fresh install, the packaged core is
registered under `/usr/lib/mihomo-tui/cores/<version>` and selected by `current`. The first TUI run
creates or imports `/etc/mihomo-tui/config.yaml`; pressing `p` explicitly validates the config and
starts or reloads the managed service.

A fresh package install stops before unpacking if it finds existing Mihomo or mihomo-tui binaries,
a managed-core root, or a Mihomo systemd unit/drop-in in the paths owned by the bundle. Remove the
conflicting local installation first, or keep it and run the standalone TUI in external-controller
mode. Package upgrades skip this fresh-install guard and preserve the selected managed core.

For a paired package/core update, install the reviewed new Deb first, inspect both versions, then
activate explicitly:

```bash
sudo apt install ./mihomo-tui_NEW_VERSION.deb
mihomo-tui core status
sudo mihomo-tui core upgrade
```

Package upgrade registers the new core but preserves the active core and rollback binary. The
explicit upgrade validates the already installed candidate, atomically switches `current`, restarts
Mihomo, checks `/version` and `/proxies`, and restores the previous core if activation fails.

See [`docs/debian-package.md`](docs/debian-package.md) for packaging, removal, publication, and
license gates.

## Run From Source

Build the program, then run it as root when using the default system paths:

```bash
cargo build --release
sudo ./target/release/mihomo-tui
```

Startup does not install, start, validate, or reload Mihomo. On first use, the program imports
`/etc/mihomo/config.yaml` into `/etc/mihomo-tui/config.yaml`. If no legacy config exists, it creates
a minimal native config. Once the owned config exists, the legacy file is never read again.

The owned file is directly usable by Mihomo and retains fields mihomo-tui does not understand:

```yaml
mixed-port: 7890
external-controller: 127.0.0.1:9093
rules:
  - MATCH,DIRECT
```

Edits are written directly to `/etc/mihomo-tui/config.yaml`. Press `p` to validate that same file,
configure `mihomo.service` to use `/etc/mihomo-tui` as its data directory, reload the core, and
verify HTTP subscriptions. Neither editing nor applying writes `/etc/mihomo/config.yaml`.

Early mihomo-tui builds used a `kind/backend/profile` wrapper. Startup automatically migrates that
wrapper to native Mihomo YAML and keeps a private backup.

When running only the Rust binary from source on a clean Debian/Ubuntu host, the first explicit apply
can download the pinned official `v1.19.29` package, verify its SHA-256 and Deb metadata, install it
through `dpkg`, configure the runtime service, and then load the single config. Downloaded artifacts
stay root-owned from creation through installation.

Automatic installation currently supports `x86_64` and `aarch64` Debian/Ubuntu systems running
systemd. It never upgrades an existing installation and refuses to overwrite a partial installation
where only the binary or service exists.

## Managed Core Policy

Mihomo remains a separate process, but its tested release contract is part of the product. The
embedded [`managed-core.json`](managed-core.json) manifest is the only source of package names,
architectures, versions, license hash, and SHA-256 hashes used by the installer and bundle builder.
The current contract recommends `v1.19.29` and accepts installed versions from `v1.19.28` up to, but
not including, `v1.20.0` for locally managed apply.

Pressing `p` checks an existing local core against that tested range before validating or reloading
the configuration. A too-old or untested-newer core is left untouched and produces an actionable
error. Existing cores are never silently upgraded, and normal startup never checks for or installs a
new upstream release.

Upstream versions enter `mihomo-tui` through an automated proposal branch and reviewed pull request.
The proposal fails closed outside the compatibility range and must pass Rust tests, Clippy, native
amd64/arm64 package builds, archive inspection, disposable-runner installation, and transaction
rollback tests. Normal startup and `p` never run `core upgrade`.

Inspect or explicitly upgrade the managed core:

```bash
mihomo-tui core status
sudo mihomo-tui core upgrade
```

The complete contract is documented in [`docs/managed-core.md`](docs/managed-core.md), and the
architectural rationale is recorded in
[`ADR-0001`](docs/decisions/0001-bundle-mihomo-as-a-separate-process.md).

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

`--workspace` / `MIHOMO_TUI_CONFIG` selects the single owned config. `--config` /
`MIHOMO_CONFIG` selects a legacy config used only when the owned config does not exist. Supplying an
explicit legacy config selects external mode and never manages local systemd.

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

An explicit `--controller`, `MIHOMO_CONTROLLER`, `--config`, `MIHOMO_CONFIG`, `--workspace`, or
`MIHOMO_TUI_CONFIG` uses external mode and disables local systemd management. Offline edits remain
available. An explicit controller never inherits the secret from the owned config; provide
`--secret` or `MIHOMO_SECRET` for that controller.

Keys:

- `1`, `2`, `3`, `4`: switch pages
- `Tab`: next page
- `p`: validate the single config and reload the managed Mihomo core
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
- `s`: save rule changes to the single configuration
- `q` or `Esc`: quit

File providers have no remote subscription URL, so `r` cannot download a new source for them.
Update the configured source file or replace the provider with an HTTP provider that has a URL.

The complete migration, storage, and apply contract is documented in
[`docs/independent-config.md`](docs/independent-config.md).
