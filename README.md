# mihomo-tui

An SSH-friendly terminal control center for [Mihomo](https://github.com/MetaCubeX/mihomo).

The project deliberately keeps Mihomo as the proxy core and focuses on the Linux workflow around it: profiles, proxy groups, rules, TUN, DNS, and service diagnostics.

## Current MVP

- Ratatui dashboard with Status, Proxies, Rules, and Config pages
- Demo mode when no controller is supplied
- Mihomo `/proxies` API discovery and refresh
- Proxy-group selection through the controller API
- Reads and edits the active Mihomo YAML for rules, providers, TUN, DNS, port, and mode
- TUN settings for stack, device, routes, DNS hijacking, MTU, and excluded networks
- DNS settings for listen address, enhanced mode, Fake IP ranges, IPv6, HTTP/3, and routing rules
- Custom rule creation at the top or bottom, editing, removal marking, and priority reordering
- Candidate validation, permission-preserving writes, backups, service reload, and automatic rollback
- ASCII-compatible UI for SSH terminals without Nerd Fonts

## Run

```bash
cargo run
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
