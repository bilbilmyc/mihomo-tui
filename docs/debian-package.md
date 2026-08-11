# Debian Bundle Operations

## Scope

The `mihomo-tui` Deb is the standalone distribution for native amd64 and arm64 Debian/Ubuntu
systemd hosts. It ships one Rust TUI and one reviewed official Mihomo core while keeping Mihomo as a
separate process.

The package conflicts with the upstream `mihomo` package because both provide `mihomo.service`.
External-controller mode remains available when Mihomo is managed outside this package.

## Build And Verify

Install a native Rust toolchain and the Debian build tools used by `scripts/build-deb.sh`, including
`curl`, `jq`, `file`, `dpkg-dev`, and `systemd` utilities. Build only on the target architecture:

```bash
./scripts/build-deb.sh --architecture "$(dpkg --print-architecture)"
./scripts/test-deb-package.sh dist/*.deb
```

The builder always compiles a fresh default `target/release/mihomo-tui` with `--locked`. An explicit
`--binary PATH` is accepted for a separately controlled native build, but its version, executable
type, symlink status, and architecture are still checked.

The builder performs these trust-boundary checks:

- accepts only the canonical release, asset, architecture, Deb version, hashes, and GPL-3.0 license
  metadata in `managed-core.json`;
- limits the core Deb to 128 MiB and the license file to 1 MiB;
- verifies package SHA-256, `Package`, `Version`, and `Architecture` before extraction;
- extracts and executes only the exact regular `usr/bin/mihomo` candidate;
- requires the core's reported version to equal the recommended manifest version;
- derives `libc6` and `libgcc-s1` requirements from the actual Rust ELF with `dpkg-shlibdeps`;
- builds the archive with `dpkg-deb --build --root-owner-group` and writes a mode-0644 checksum.

`scripts/test-deb-package.sh` unpacks without installing and verifies metadata, payload paths,
permissions, unit syntax, native executable versions, license hash, source notice, and checksum.

## Installed Layout

```text
/usr/bin/mihomo-tui
/usr/lib/mihomo-tui/bundled/<version>/mihomo   package-owned payload
/usr/lib/mihomo-tui/cores/<version>/mihomo     managed immutable hard link
/usr/lib/mihomo-tui/current                    active relative symlink
/usr/lib/systemd/system/mihomo.service
/usr/share/doc/mihomo-tui/Mihomo-LICENSE
/usr/share/doc/mihomo-tui/Mihomo-NOTICE
/usr/share/doc/mihomo-tui/server-guide.md
/etc/mihomo-tui/config.yaml                    runtime-created native config
```

`postinst` validates root ownership and non-writable parent modes before hard-linking the payload
into `cores`. It never overwrites different bytes for an existing version. The hard link avoids a
second copy of the core and keeps the managed inode alive when dpkg removes an older package payload.

The package does not own `current` or `cores/<version>/mihomo`. This is intentional: installing a
new package registers a candidate but does not delete the active rollback core or switch activation.

## Fresh Install

Use APT so declared runtime dependencies are resolved:

```bash
sudo apt install ./dist/mihomo-tui_*.deb
mihomo-tui core status
sudo mihomo-tui
```

Package installation runs `systemctl daemon-reload` when systemd is active, but it does not enable
or start `mihomo.service`. On a clean host, `postinst` selects the bundled version because no
`current` link exists. Running the TUI initializes `/etc/mihomo-tui/config.yaml`; pressing `p` is the
explicit action that validates the config and starts or reloads the service.

Before unpacking a fresh install, `preinst` rejects existing Mihomo or mihomo-tui binaries, the
managed-core root, and common Mihomo unit, drop-in, or enable-symlink paths under `/etc`, `/run`,
`/usr/lib`, and `/lib`. This prevents the bundle from taking ownership of an unmanaged installation.
The guard does not run for package upgrades, where those paths already belong to this package.

## Package And Core Upgrade

Installing a reviewed newer bundle is preparation, not activation:

```bash
sudo apt install ./mihomo-tui_NEW_VERSION.deb
mihomo-tui core status
sudo mihomo-tui core upgrade
```

Before `core upgrade`, the owned config must be a trusted root file and define
`external-controller`; the packaged unit must be the loaded `mihomo.service`; and the current core
must remain inside the tested compatibility range.

The explicit upgrade takes the shared root lock, reuses the package-registered candidate when
present, validates the config with that exact binary, atomically switches `current`, restarts the
service, and polls both `/version` and `/proxies`. Any activation or health failure switches back to
the previous version, restarts it, checks rollback health, and reports both errors if rollback also
fails. Normal startup and configuration apply never invoke this transaction.

## Removal

```bash
sudo apt remove mihomo-tui
```

Removal disables and stops `mihomo.service`, removes the package payload, active link, and managed
core binaries, then reloads systemd. Runtime-created `/etc/mihomo-tui/config.yaml` and its backups
are not package-owned and are not deleted automatically. Remove those files separately only when
their configuration and credentials are no longer needed.

## CI And Publication

Every pull request runs Rust formatting, tests, Clippy, RustSec, bundle policy tests, and native
amd64/arm64 package builds. Each native runner unpacks the Deb, installs it on the disposable host,
asserts the service remains disabled and inactive, checks managed layout discovery, simulates package
payload replacement, verifies active-core preservation, and purges the package.

The scheduled upstream workflow can update only release/package/license hashes inside the existing
compatibility range. It pushes a proposal branch, runs the same two-architecture gates, and opens a
pull request. It never installs on a user host, widens compatibility, merges, or publishes a release.

Before publishing artifacts outside CI, maintainers must:

- review the manifest diff and upstream release notes;
- confirm both architecture jobs and rollback tests passed;
- verify `.deb` and `.sha256` artifact names and hashes;
- retain `Mihomo-LICENSE`, `Mihomo-NOTICE`, and the corresponding source URL;
- declare the mihomo-tui project's own license and copyright holder;
- replace the placeholder package maintainer identity with a real contact;
- require a human approval for the paired release.

The Mihomo payload is recorded as GPL-3.0 with a pinned license hash. That metadata does not grant or
choose a license for the Rust `mihomo-tui` source. Public release remains blocked until the project
maintainers make that separate legal decision.
