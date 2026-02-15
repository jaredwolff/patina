# Release Engineering

## CI

`/.github/workflows/ci.yml` runs on pushes to `main` and pull requests:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`
- Release-binary smoke checks (`patina --help`, `patina status`)

## Tagged Builds

`/.github/workflows/release.yml` runs on:

- tags matching `v*` (for example `v0.1.0`)
- manual `workflow_dispatch`

It builds `patina` on Linux, macOS, and Windows and uploads archive artifacts with SHA256 checksums.

## Local Packaging

Use the packaging script from repo root:

```bash
./scripts/package-release.sh v0.1.0
```

Optional explicit target triple:

```bash
./scripts/package-release.sh v0.1.0 x86_64-unknown-linux-gnu
```

Artifacts are written to `dist/`:

- `patina-bot-<version>-<platform>.tar.gz`
- `patina-bot-<version>-<platform>.sha256`
