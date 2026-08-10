# Managed Mihomo Core

## Objective

`mihomo-tui` ships and manages a tested official Mihomo core while keeping the core in a separate
process. Users get one product and one configuration workflow without turning this repository into
a Mihomo source fork or linking the Go core into the Rust process.

The managed-core contract has two consumers:

- local managed mode installs, validates, configures, and reloads the tested core;
- external mode keeps using a user-managed controller and never changes the local runtime.

The first delivery phase moves release metadata and compatibility policy into one embedded manifest,
then makes the existing installer consume that contract. Explicit core upgrades, staged health checks,
rollback, release automation, and bundled distribution artifacts follow in later phases.

## Tech Stack

- Rust 2024 binary with Clap, Ratatui, Reqwest, Serde, and SHA-256 verification.
- Official Mihomo release binaries remain separate operating-system processes.
- An embedded JSON manifest is the single source of truth for the recommended version, tested
  compatibility range, target packages, package metadata, and hashes.
- Local lifecycle management remains limited to supported Debian/Ubuntu systemd hosts.

## Commands

```bash
cargo test --all-targets
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --release
sudo ./target/release/mihomo-tui
```

## Project Structure

```text
managed-core.json       Embedded, reviewable official-core release contract
src/core.rs             Manifest parsing, validation, version parsing, compatibility policy
src/runtime.rs          Privileged installation and systemd lifecycle orchestration
src/mihomo.rs           External Controller API boundary
docs/managed-core.md    Architecture, security boundaries, and delivery phases
```

## Code Style

Use explicit result types at trust boundaries and keep policy pure so it can be tested without root,
network, or filesystem access:

```rust
pub fn compatibility(version: CoreVersion) -> Compatibility {
    if version < minimum {
        Compatibility::TooOld
    } else if version >= maximum_exclusive {
        Compatibility::UntestedNewer
    } else {
        Compatibility::Supported
    }
}
```

Release metadata belongs in the manifest. Runtime code must not duplicate version strings, package
names, architectures, or hashes.

## Testing Strategy

- Small unit tests validate manifest structure, supported targets, version output parsing, and every
  compatibility boundary.
- Existing runtime tests continue covering URL allowlists, size caps, SHA-256 verification, deb
  metadata, systemd ownership checks, and install planning.
- The full Rust test suite, formatter, Clippy with warnings denied, and release build are required for
  every manifest or runtime change.
- Before enabling upgrades, a disposable Debian/Ubuntu VM test must cover install, config validation,
  restart, Controller API health, failed-health rollback, and restart after rollback.

## Boundaries

Always:

- use the official `MetaCubeX/mihomo` GitHub release path over HTTPS;
- cap downloads, restrict redirects, verify SHA-256, and verify deb package metadata;
- require an explicit user action before installing or upgrading a core;
- stage and validate a candidate before changing the active core;
- retain the previous working core until the replacement passes health checks;
- keep external-controller mode free of local runtime mutations.

Ask first:

- expanding supported operating systems, service managers, architectures, or release hosts;
- changing the tested compatibility range without integration-test evidence;
- changing the core installation layout or taking ownership of an existing unmanaged service;
- adding automatic upgrade execution to normal TUI startup.

Never:

- copy Mihomo source into this repository or expose it through in-process FFI;
- silently follow the latest upstream release;
- run a downloaded artifact before integrity and package metadata checks pass;
- overwrite a partial or unmanaged installation;
- display or log Controller secrets or subscription credentials.

## Delivery Phases

### Phase 1: Version Contract

- Embed one validated managed-core manifest.
- Select packages by operating system and architecture through that manifest.
- Parse the installed core version and require it to be in the tested compatibility range before a
  locally managed apply.
- Keep the current behavior of never upgrading an existing installation.

### Phase 2: Explicit Staged Upgrade

- Add a separate explicit upgrade command or TUI confirmation flow.
- Download and validate the candidate without replacing the active binary.
- Validate the owned config with the candidate, activate atomically, restart, and check `/version`
  and `/proxies`.
- Restore the previous core automatically when activation or health checks fail.

### Phase 3: Release Synchronization

- Detect new official Mihomo releases in CI.
- Generate a manifest-update pull request with package metadata and hashes.
- Run the architecture build matrix and disposable-host integration tests.
- Require human approval before publishing a new paired `mihomo-tui` release.

### Phase 4: Bundled Distribution

- Publish checksummed Debian/Ubuntu artifacts containing `mihomo-tui`, the tested official core, the
  service definition, license notices, and install/uninstall scripts.
- Keep external mode available for users who manage Mihomo separately.

## Success Criteria

Phase 1 is complete when:

- one manifest is the only source of recommended version, compatible range, package names, package
  versions, architectures, and hashes;
- malformed or incomplete embedded release metadata fails closed;
- installed core version output is parsed without substring matching;
- local apply accepts tested versions and rejects too-old or untested-newer versions with actionable
  errors;
- external mode and clean-host install behavior remain unchanged;
- all verification commands pass without changing the running host service.

## Open Questions

- The exact CLI/TUI confirmation design for Phase 2 will be specified before upgrade code is added.
- The final filesystem layout for bundled cores will be decided together with Debian packaging.
- License notices and source-offer requirements will be verified against the Mihomo version bundled
  by each release.
