# Independent Configuration

## Goal

`mihomo-tui` owns a versioned configuration source that can be opened and edited
without Mihomo being installed, running, or reachable. Mihomo is an optional
runtime backend and is only required when the user explicitly applies the
configuration.

## Files And Ownership

- The default source is `/etc/mihomo-tui/config.yaml`.
- The default Mihomo runtime target is `/etc/mihomo/config.yaml`.
- `mihomo-tui` reads and edits the source during normal use.
- Mihomo reads the runtime target.
- Normal edits never write the runtime target, reload a service, install Mihomo,
  or call the Mihomo executable.

The source format is:

```yaml
kind: mihomo-tui/v1
backend: mihomo
profile:
  # Complete Mihomo YAML mapping
```

The complete Mihomo document lives under `profile`. Import and subsequent edits
must preserve fields that `mihomo-tui` does not understand.

## First-Run Migration

When the source does not exist, initialization follows this order:

1. Import an explicitly selected legacy/runtime config when one was supplied.
2. Otherwise import the default runtime target when it exists.
3. Otherwise create a minimal editable profile.

Initialization creates the source atomically with mode `0600`. It never replaces
an existing source, even when a newer or different runtime target exists. An
import copies the full YAML mapping into `profile`; it does not move, rewrite, or
delete the original file.

Malformed YAML, a non-mapping legacy root, an unsupported source `kind`, or an
unsupported `backend` stops initialization with an actionable error. No partial
source is left behind.

## Offline Editing

Configuration reads accept the versioned source format. Legacy raw Mihomo YAML
remains readable for compatibility with explicit/external workflows and tests.
All editor operations update only the profile mapping and preserve the wrapper
metadata and unmanaged profile fields.

Each edit is a syntax-checked atomic write. The previous source is retained as a
mode-`0600` backup. Editing has no dependency on a Mihomo binary or controller.

The UI reports whether the source profile equals the runtime target:

- `已应用`: the normalized YAML values are equal.
- `待应用`: the target is missing, unreadable, or differs from the source.

## Explicit Apply

Applying is a separate user command. For the managed local backend it performs:

1. Read and validate the source wrapper.
2. Serialize only `profile` as a candidate runtime config.
3. Ensure the managed Mihomo runtime is available when automatic installation is
   enabled.
4. Validate the candidate with the trusted Mihomo executable.
5. Atomically replace the runtime target while retaining a mode-`0600` backup.
6. Reload or restart `mihomo.service`.
7. Refresh configured HTTP providers and verify that each returns nodes.

If candidate validation fails, the runtime target is unchanged. If the service
reload fails, the target is rolled back and the restored version is reloaded.
Source edits are never rolled back by an apply failure.

External controller mode does not manage a local service. Its source remains
editable, but local apply is unavailable unless a managed runtime target was
explicitly configured.

## Command-Line Contract

- `--workspace` / `MIHOMO_TUI_CONFIG` selects the independent source.
- `--config` / `MIHOMO_CONFIG` selects a legacy/runtime config to import on first
  use and the runtime target for explicit apply.
- `--controller` / `MIHOMO_CONTROLLER` and `--secret` / `MIHOMO_SECRET` override
  controller discovery without changing either file.
- `--no-auto-install` prevents installation during apply. It does not affect
  startup or offline editing because those operations never install Mihomo.

## Acceptance Criteria

- The TUI opens and permits subscription, rule, TUN, and DNS edits while Mihomo
  is stopped, absent, or unreachable.
- A first import preserves unknown nested fields and leaves the legacy file byte
  for byte unchanged.
- An existing source is never overwritten by initialization.
- Source files and backups are mode `0600` on Unix.
- Saving a new subscription URL succeeds without invoking Mihomo and remains
  present after reopening the TUI.
- The runtime target does not change until the explicit apply command.
- Validation or reload failure cannot leave a broken runtime target in place.
- The UI exposes source path, runtime path, and applied/pending state without
  displaying subscription credentials or the API secret.
