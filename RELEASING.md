# Releasing

This document describes the maintainer release process for `ripdiff`.

## Versioning

`ripdiff` follows [Semantic Versioning](https://semver.org/) (`MAJOR.MINOR.PATCH`):

- `PATCH` for bug fixes and internal improvements.
- `MINOR` for backwards-compatible features.
- `MAJOR` for breaking changes.

## Release Workflow

Releases are driven by git tags.

The GitHub Actions workflow in [.github/workflows/release.yml](/home/mabeleda/Development/ripdiff/.github/workflows/release.yml#L1) runs when a tag matching `v*` is pushed. It:

1. Checks that the tag version matches `Cargo.toml`.
2. Verifies the crate can be packaged.
3. Publishes the crate to crates.io.
4. Creates a GitHub release for the tag.

To publish from CI, the repository must have a `CARGO_REGISTRY_TOKEN` secret containing a crates.io API token.

## Recommended Process

Use [`cargo-release`](https://github.com/crate-ci/cargo-release) to bump the crate version and create the matching tag in one step.

Releases should only be cut from `main`. This keeps each published version tied to the reviewed, canonical branch state and avoids accidentally releasing from an unmerged branch or local-only commit. The repository's `cargo-release` configuration enforces this with `allow-branch = ["main"]`.

Install it once:

```bash
cargo install cargo-release
```

Preview a release:

```bash
cargo release patch
```

Run a release:

```bash
cargo release patch --execute
```

Replace `patch` with `minor` or `major` as needed.

With the repository configured for `cargo-release`, the executed command should:

1. Update `Cargo.toml` to the next version.
2. Create a release commit.
3. Create a matching tag like `v0.1.1`.
4. Push the commit and tag to `origin`.

After the tag is pushed, GitHub Actions performs the actual crates.io publish and creates the GitHub release.

## Maintainer Checklist

1. Ensure CI is green locally or on `main`:
   ```bash
   cargo fmt
   cargo clippy --all-targets --all-features
   cargo test
   cargo build
   ```
2. Run a dry run:
   ```bash
   cargo release patch
   ```
3. Execute the release:
   ```bash
   cargo release patch --execute
   ```
4. Verify the GitHub Actions release workflow succeeds.
5. Verify the new version appears on crates.io.
