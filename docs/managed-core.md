# Managed Mihomo Core

## Objective

`mihomo-tui` ships and manages a tested official Mihomo core while keeping the core in a separate
process. Users get one product and one configuration workflow without turning this repository into
a Mihomo source fork or linking the Go core into the Rust process.

The managed-core contract has two consumers:

- local managed mode installs, validates, configures, and reloads the tested core;
- external mode keeps using a user-managed controller and never changes the local runtime.

Release metadata, compatibility policy, license identity, package construction, explicit activation,
health rollback, and upstream synchronization all consume the same embedded manifest. Normal TUI
startup never checks upstream and never activates a newly available core.

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
./target/release/mihomo-tui core status
sudo ./target/release/mihomo-tui core upgrade
```

## Project Structure

```text
managed-core.json       Embedded, reviewable official-core release contract
src/core/               Version parsing, manifest validation, and compatibility policy
src/system.rs           Trusted root files, sanitized commands, and private temporary paths
src/core_package/       Download, integrity, Deb metadata, extraction, and secure temporary files
src/core_manager/       Managed-core facade, inventory, immutable storage, and tests
src/core_upgrade/       Candidate orchestration, activation, API health checks, and rollback
src/runtime/            Runtime facade, installation, systemd lifecycle, and tests
src/mihomo.rs           External Controller API boundary
packaging/debian/       Unit and maintainer-script templates
scripts/build-deb.sh    Native standalone bundle builder
docs/managed-core.md    Architecture, security boundaries, and operations
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
- Disposable native runners install and remove each architecture bundle, assert that the service
  remains disabled and inactive, verify managed layout discovery, and prove package reinstall does
  not replace the operator-selected active core.
- Fake activation operations cover successful restart/health, failed-health rollback, rollback
  health, and combined activation/rollback errors without mutating the test host.

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

## Managed-Core Lifecycle

### Release Contract

- Embed one validated managed-core manifest.
- Select packages by operating system and architecture through that manifest.
- Parse the installed core version and require it to be in the tested compatibility range before a
  locally managed apply.
- Keep normal startup and configuration apply free of upgrade behavior.

### Explicit Staged Upgrade

- `mihomo-tui core status` reports the installed, active, recommended, and compatible versions without
  changing the host.
- `sudo mihomo-tui core upgrade` is the only upgrade entry point. Normal startup and `p` never invoke
  it.
- Managed binaries live at `/usr/lib/mihomo-tui/cores/<version>/mihomo`. The root-owned
  `/usr/lib/mihomo-tui/current` symlink selects one immutable version directory.
- The systemd drop-in starts `/usr/lib/mihomo-tui/current/mihomo -d /etc/mihomo-tui`.
- Upgrade reuses an already installed recommended candidate when available. Otherwise it downloads
  and verifies the official Deb, extracts only its Mihomo binary into a private staging directory,
  verifies its exact version, and validates the owned config with that candidate.
- Before first activation, the current trusted binary is copied into its own version directory so it
  is always available for rollback.
- Activation atomically replaces the `current` symlink, reloads systemd, restarts the service, then
  checks `/version` and `/proxies` through the controller discovered from the owned config.
- Any activation, restart, version, or proxy health failure atomically restores the previous symlink,
  restarts the previous core, verifies rollback health, and returns an error describing both failures
  when rollback is also unhealthy.
- A successful upgrade retains the previous core for automatic rollback evidence and operator
  inspection, and removes unrelated staging files. Re-running upgrade at the recommended version is
  an idempotent no-op.

### Release Synchronization

- Detect the latest published, non-draft, non-prerelease Mihomo release through GitHub's documented
  `GET /repos/MetaCubeX/mihomo/releases/latest` endpoint.
- Generate a manifest-update branch and pull request with package metadata and hashes. The workflow
  receives only `contents: write` and `pull-requests: write` permissions.
- Run the architecture build matrix and disposable-host integration tests.
- Require human approval before publishing a new paired `mihomo-tui` release.
- Patch releases inside the existing compatibility range can be proposed automatically. A release
  outside that range fails closed and requires a reviewed compatibility-policy change.

### Bundled Distribution

- Build Debian packages with `dpkg-deb --build --root-owner-group`. The package contains
  `mihomo-tui`, the tested official core under `bundled/<version>`, the `mihomo.service` unit,
  license/source notices, and maintainer scripts.
- On install, `postinst` creates a hard link at `cores/<version>/mihomo` after validating root
  ownership, permissions, and immutable same-version bytes. It creates `current` only when no active
  link exists. The managed hard link is deliberately not owned by dpkg, so replacing a package
  payload cannot delete the active or rollback core.
- Installing a newer bundle registers its core but preserves `current`. Activation remains an
  explicit `sudo mihomo-tui core upgrade` transaction.
- The bundle conflicts with a separately packaged `mihomo` because both would own the same service;
  users must explicitly choose bundled or externally managed mode.
- Publish SHA-256 checksums for every architecture artifact.
- Keep external mode available for users who manage Mihomo separately.

## Upgrade Transaction

```text
lock -> inspect current -> reuse installed candidate or download/hash/deb/extract
     -> candidate version/config checks -> stage immutable version
     -> atomically switch current -> daemon-reload -> restart -> API health
     -> success: retain old version
     -> failure: atomically switch old current -> restart -> rollback health
```

Every path before the atomic link switch is side-effect free outside private staging and an optional
new immutable version directory. The active service is never stopped merely to download or inspect a
candidate.

## Authoritative Sources

- GitHub latest release API: <https://docs.github.com/en/rest/releases/releases#get-the-latest-release>
- GitHub Actions workflow syntax and permissions:
  <https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax>
- GitHub CLI pull-request creation: <https://cli.github.com/manual/gh_pr_create>
- Debian archive operations use the installed `dpkg-deb` interface (`--field`, `--extract`, and
  `--build --root-owner-group`) and verify its availability as a trusted root executable.

## Success Criteria

The implementation is ready for a reviewed release when:

- one manifest is the only source of recommended version, compatible range, package names, package
  versions, architectures, and hashes;
- malformed or incomplete embedded release metadata fails closed;
- installed core version output is parsed without substring matching;
- local apply accepts tested versions and rejects too-old or untested-newer versions with actionable
  errors;
- external mode and clean-host install behavior remain unchanged;
- upgrade is explicit, staged, health-checked, idempotent, and rollback-tested;
- upstream synchronization creates reviewable PRs and cannot widen compatibility automatically;
- both supported architecture packages build, contain the expected files, pass metadata inspection,
  install on disposable runners, and publish checksums;
- all verification commands pass without changing the running host service.

## Release Gate

No bundle is published until the bundled Mihomo license text and corresponding source URL are
present, the disposable-host package tests and rollback tests pass, and a human approves the release
PR. The repository must also declare the mihomo-tui project's own license and a real package
maintainer identity before public distribution; the current automated checks cover the bundled
Mihomo GPL-3.0 material but cannot choose a license or copyright holder for this project. Absence of
either project's license material, source information, rollback evidence, or architecture coverage
blocks release.
