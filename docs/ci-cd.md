# CI/CD — Release Pipeline

Automated cross-platform builds via GitHub Actions.

## Workflows

### Auto-tag (`.github/workflows/auto-tag.yml`)

Triggers on every push to `main`. Reads the `version` field from `Cargo.toml` and creates a `v<version>` git tag if it doesn't already exist.

### Release (`.github/workflows/release.yml`)

Triggers on tag push (`v*`). Builds the plugin for all four platforms, packages them, and creates a GitHub Release with downloadable binaries.

## Build Matrix

| Target | OS Runner | Artifact |
|--------|-----------|----------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `se-ai-translator-linux-x86_64.tar.gz` |
| `x86_64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-x86_64.tar.gz` |
| `aarch64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-aarch64.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `se-ai-translator-windows-x86_64.zip` |

## Release Flow

1. Merge PR to `main`
2. Auto-tag workflow creates `v<version>` tag (from `Cargo.toml`)
3. Release workflow fires on the new tag
4. Four parallel jobs build for each target
5. A release job collects all artifacts and creates a GitHub Release

## Version Bumping

1. Update `version` in `Cargo.toml`
2. Commit and push/merge to `main`
3. The tag and release happen automatically

The release workflow verifies the git tag matches `Cargo.toml` version — a mismatch fails the build.

## Linux Dependencies

eframe/egui requires these system libraries on Ubuntu. The workflow installs them automatically:

```
libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
libspeechd-dev libxkbcommon-dev libssl-dev pkg-config
```

## Testing Locally

```bash
bash tests/run_test.sh --dev gui     # Quick dev build test
bash tests/run_test.sh gui           # Full release build test
```

See `tests/README.md` or `tests/run_test.sh --help` for more test options.
