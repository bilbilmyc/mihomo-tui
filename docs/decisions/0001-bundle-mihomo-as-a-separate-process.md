# ADR-0001: Bundle Mihomo As A Separate Process

## Status

Accepted

## Date

2026-08-10

## Context

`mihomo-tui` needs to work as one installable application rather than requiring users to assemble a
Rust TUI, a compatible Mihomo binary, a service unit, and matching configuration by hand. At the
same time, Mihomo is an independently released Go project with its own lifecycle, license, runtime
state, and Controller API.

The design must support offline configuration editing, explicit privileged operations, safe core
updates, rollback, amd64 and arm64 packages, and continued use of externally managed Mihomo
controllers. Upstream updates must not silently expand the tested compatibility range.

## Decision

Distribute a single Debian package containing `mihomo-tui` and a pinned official Mihomo release, but
run Mihomo as a separate operating-system process.

Use `managed-core.json` as the reviewable source of truth for the upstream repository, recommended
version, compatibility range, architecture-specific Deb assets, Deb metadata, SHA-256 hashes, and
license hash. Runtime and packaging code fail closed when that contract is malformed.

Store active and rollback binaries in immutable version directories under
`/usr/lib/mihomo-tui/cores`. Select one version with an atomically replaced relative `current`
symlink. Package payloads live under `bundled`; `postinst` hard-links them into the managed layout so
dpkg can replace payloads without deleting an active or rollback inode.

Make core activation a separate explicit command. `core upgrade` validates the candidate and owned
config, switches atomically, restarts Mihomo, checks the Controller `/version` and `/proxies`
endpoints, and restores the previous version on failure. Installing a package registers a candidate
but does not switch an existing `current` link or start the service.

Use scheduled automation only to propose manifest updates inside the existing compatibility range.
The proposal must pass native package builds and disposable-host tests before a human reviews it.

## Alternatives Considered

### Link Or Port Mihomo Into The Rust Process

Rejected. It would turn this repository into a source fork or FFI integration, couple Rust releases
to Go internals, enlarge the privilege and crash boundary, and make external-controller mode harder
to preserve. The stable Controller API already provides the required process boundary.

### Require A Separately Installed Mihomo Package

Rejected as the primary distribution. It keeps the TUI technically dependent on user-managed
version selection and service layout, so it does not deliver one independently installable product.
External mode remains available for users who intentionally want that ownership split.

### Follow The Latest Upstream Release At Runtime

Rejected. Runtime latest checks would make normal startup nondeterministic and could activate an
untested core. Compatibility changes require code review and architecture evidence, not a successful
HTTP request.

### Activate A New Core During Package Upgrade

Rejected. Dpkg installation cannot perform the full Controller health transaction safely and would
bypass automatic rollback. Package upgrade therefore stages/registers a candidate; activation is a
separate explicit command.

## Consequences

- Users receive one Deb with the TUI, core, unit, checksums, and upstream license/source material.
- Mihomo crashes and upgrades remain isolated from the TUI process.
- Package and core versions can move together through reviewed pull requests without becoming one
  source tree or one process.
- Old managed cores consume directory entries but hard links avoid duplicating package payload data.
- Managed local mode requires root, systemd, trusted filesystem ownership, and a Controller address
  in the owned config; external mode remains non-mutating.
- Maintainers must track both projects' license obligations and cannot publish until the
  mihomo-tui project's own license and maintainer identity are declared.
