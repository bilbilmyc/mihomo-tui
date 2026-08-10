# Single Configuration

## Goal

`mihomo-tui` owns exactly one configuration file:

```text
/etc/mihomo-tui/config.yaml
```

The file is native Mihomo YAML. Both `mihomo-tui` and the managed Mihomo service
read the same file. There is no generated runtime copy and no second
mihomo-tui-owned profile.

Mihomo remains an optional runtime backend. The TUI can open and edit the file
while Mihomo is stopped, missing, or unreachable.

## File Ownership

- Normal reads and edits use `/etc/mihomo-tui/config.yaml`.
- Subscription, rule, TUN, and DNS changes are written directly to that file.
- Source backups are created beside it with mode `0600`.
- The managed Mihomo service uses `/etc/mihomo-tui` as its data directory, so it
  reads the same `config.yaml` directly.
- `/etc/mihomo/config.yaml` is not synchronized, generated, or updated.

Unknown and custom Mihomo fields must survive every edit.

## First-Run Migration

When `/etc/mihomo-tui/config.yaml` does not exist, initialization follows this
order:

1. Import an explicitly selected legacy config when one was supplied.
2. Otherwise import `/etc/mihomo/config.yaml` when it exists.
3. Otherwise create a minimal native Mihomo config.

Import copies the complete YAML mapping without changing or deleting the legacy
file. Once the new file exists, initialization never imports again.

Early builds stored this wrapper:

```yaml
kind: mihomo-tui/v1
backend: mihomo
profile:
  # Mihomo YAML
```

Initialization automatically replaces that wrapper with its complete `profile`
mapping and keeps a private backup. This is a one-time format migration.

The config file is created atomically with mode `0600`. A malformed YAML file,
non-mapping root, unsupported wrapper, symlink source, or symlink config
directory stops initialization without leaving a partial file.

## Offline Editing

Every editor operation parses the native YAML, changes only its owned fields,
preserves unmanaged fields, and atomically replaces the same config file. The
previous version is retained as a mode-`0600` backup.

Editing never installs or starts Mihomo and never calls the Mihomo executable.
The UI reports that the file is saved and, for a managed local backend, whether
the running core still needs to reload it.

## Apply And Reload

Pressing `p` is a runtime action, not a file synchronization action. For the
managed local backend it performs:

1. Prepare or optionally install Mihomo without starting an existing stopped
   service on another config.
2. Require an existing core to be inside the compatibility range embedded in
   `managed-core.json`. A clean-host install must exactly match the recommended
   version and package metadata in that manifest.
3. Validate `/etc/mihomo-tui/config.yaml` with the trusted Mihomo executable.
4. Install a root-owned systemd drop-in that sets Mihomo's data directory to
   `/etc/mihomo-tui`.
5. Reload systemd and run `systemctl reload-or-restart mihomo.service`.
6. Refresh configured HTTP providers and verify that they return nodes.

Validation failure leaves the saved config available for correction and does
not reload the service. The managed drop-in is the only supported override;
unknown service drop-ins are rejected rather than overwritten.

Startup and apply never upgrade an existing core. A new upstream Mihomo version
must first be added to the reviewed manifest and pass the managed-core release
checks described in `docs/managed-core.md`.

External controller or explicit legacy-config mode never manages local systemd.
Its config remains editable, but local apply is unavailable.

## Command-Line Contract

- `--workspace` / `MIHOMO_TUI_CONFIG` selects the single owned config. The
  default is `/etc/mihomo-tui/config.yaml`. An explicit custom path is external
  mode and never changes local systemd.
- `--config` / `MIHOMO_CONFIG` selects a legacy config used only for the first
  import. Supplying it selects external mode and never manages local systemd.
- `--controller` / `MIHOMO_CONTROLLER` and `--secret` / `MIHOMO_SECRET` override
  controller discovery without changing the config file.
- `--no-auto-install` prevents installation during `p`. It does not affect
  startup or offline editing.

## Acceptance Criteria

- The TUI opens and saves configuration while Mihomo is stopped, absent, or
  unreachable.
- Adding or replacing a subscription changes
  `/etc/mihomo-tui/config.yaml` directly.
- No edit or apply writes `/etc/mihomo/config.yaml`.
- A first import preserves unknown nested fields and leaves the imported file
  byte for byte unchanged.
- Existing wrapped workspaces migrate without losing profile fields.
- The single config and its backups are mode `0600` on Unix.
- Managed Mihomo starts and reloads with `/etc/mihomo-tui` as its data directory.
- Validation happens only when explicitly applying and cannot block offline
  editing.
- The UI does not display subscription credentials or the API secret.
