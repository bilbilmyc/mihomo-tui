# Linux Release Artifacts

## Artifact Contract

CI builds natively on x86_64 and aarch64. Each architecture job emits three artifacts and one
SHA-256 file for each artifact:

| Format | x86_64 name | aarch64 name | Contents |
| --- | --- | --- | --- |
| Deb | `mihomo-tui_<version>+mihomo<core>-1_amd64.deb` | `..._arm64.deb` | TUI, pinned core, unit, docs |
| RPM | `mihomo-tui-<version>-1.mihomo<core>.x86_64.rpm` | `...aarch64.rpm` | TUI, pinned core, unit, docs |
| Native | `mihomo-tui-<version>-linux-x86_64` | `...-linux-aarch64` | TUI ELF only |

Deb and RPM use the same `/usr/lib/mihomo-tui` managed-core layout and preserve the selected core
during package replacement. Neither package enables or starts `mihomo.service` during installation.
Both reject an unmanaged Mihomo binary, unit, or managed-core directory on a fresh install.

The native artifact is intentionally only the Rust executable. It can connect to an existing
Controller on any compatible glibc Linux system. Its managed local auto-install path remains limited
to Debian/Ubuntu because runtime package verification currently accepts official Deb metadata.

## Local Build

Build only on a native target host. The package builders reject cross-architecture binaries:

```bash
sudo apt-get install cpio curl dpkg-dev file jq rpm
cargo build --release --locked

rust_arch=$(uname -m)
deb_arch=$(dpkg --print-architecture)
./scripts/build-native.sh --architecture "$rust_arch" --binary target/release/mihomo-tui
./scripts/build-deb.sh --architecture "$deb_arch" --binary target/release/mihomo-tui
./scripts/build-rpm.sh --architecture "$rust_arch" --binary target/release/mihomo-tui
```

Verify without installing:

```bash
./scripts/test-native-binary.sh dist/mihomo-tui-*-linux-"$(uname -m)"
./scripts/test-deb-package.sh dist/*.deb
./scripts/test-rpm-package.sh dist/*.rpm
```

Disposable-host tests are destructive to a `mihomo-tui` package already installed on that host. CI
runs the Deb test on a fresh GitHub runner and the RPM test inside a fresh Fedora container:

```bash
sudo ./scripts/test-install-deb.sh dist/*.deb
docker run --rm \
  --volume "$PWD:/work:ro" \
  --workdir /work \
  fedora:42@sha256:99e203b80b1c3d8f7e161ec10a68fd02b081ef83a3963553e513c82846b97814 \
  bash -lc 'dnf install -y jq && ./scripts/test-install-rpm.sh dist/*.rpm'
```

## CI And Releases

Every pull request and branch push runs formatting, Rust tests, Clippy, RustSec, release policy tests,
and both native architecture package jobs. The package jobs build and inspect all formats, install
the Deb on the disposable runner, install the RPM in Fedora, exercise reinstall preservation, and
upload the verified files as 14-day workflow artifacts.

A tag must exactly match the Cargo version, for example `v0.1.0`. Only after all quality, security,
and package jobs pass does CI create or update a draft GitHub Release containing all 12 files. A
maintainer must review and publish that draft. Rerunning the workflow replaces existing draft assets
instead of creating duplicate releases.

The weekly managed-core proposal workflow uses the same package action. An upstream version change
therefore cannot open a pull request unless Deb, RPM, native binary, and disposable installation
checks all pass on both architectures.

## Publication Gate

CI produces private workflow artifacts and draft releases, but public publication still requires:

- a declared license and copyright holder for the mihomo-tui Rust project;
- a real Deb/RPM maintainer contact instead of the current placeholder;
- review of the pinned Mihomo release, hashes, license text, and source notice;
- successful x86_64 and aarch64 package jobs;
- human review of the draft release.

The bundled Mihomo license and source notice do not choose a license for mihomo-tui itself.
