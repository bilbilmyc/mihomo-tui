# mihomo-tui

An SSH-friendly terminal control center for [Mihomo](https://github.com/MetaCubeX/mihomo).

The project deliberately keeps Mihomo as the proxy core and focuses on the Linux workflow around it: profiles, proxy groups, rules, TUN, DNS, and service diagnostics.

## Current MVP

- Ratatui dashboard with Status, Proxies, Rules, and Config pages
- Demo mode when no controller is supplied
- Mihomo `/proxies` API discovery and refresh
- Proxy-group selection through the controller API
- Reads the active Mihomo YAML for rules, providers, TUN, DNS, port, and mode
- Rule enable/disable and priority reordering with validated save, backup, and service reload
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
- `r`: refresh Mihomo data
- `Right` or `Enter`: enter the selected proxy group's node list
- `Left`: return to proxy groups
- `Enter`: apply the selected node when the node list is focused
- `Space`: enable/disable a rule
- `J`/`K`: move a rule down/up
- `s`: save rules, validate the candidate YAML, create a backup, and reload Mihomo
- `q` or `Esc`: quit

## Planned slices

1. Profile and subscription storage with manual update and rollback
2. Rule editor with domain/IP/geosite/rule-provider helpers
3. Mihomo process lifecycle and config validation
4. TUN, DNS, system proxy, and systemd diagnostics
5. Connections, logs, traffic, and latency views
