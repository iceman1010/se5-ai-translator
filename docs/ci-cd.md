# CI/CD — Release Pipeline

Automated cross-platform builds via GitHub Actions.

## Workflows

### Auto-tag (`.github/workflows/auto-tag.yml`)

Triggers on every push to `master`. Reads the `version` field from `Cargo.toml` and creates a `v<version>` git tag if it doesn't already exist.

> **Known issue:** Tags pushed by the auto-tag workflow use `GITHUB_TOKEN`, which [cannot trigger other workflows](https://docs.github.com/en/actions/using-workflows/triggering-a-workflow#triggering-a-workflow-from-a-workflow). This means the release workflow will NOT fire automatically from auto-tag. See the workaround below.

### Release (`.github/workflows/release.yml`)

Triggers on tag push (`v*`) or manual `workflow_dispatch`. Builds the plugin for all four platforms, packages them, and creates a GitHub Release with downloadable binaries.

> **Note:** The release step (creating the GitHub Release with artifacts) only works on real tag pushes — `workflow_dispatch` has no tag ref and the release step will fail with "GitHub Releases requires a tag". Use `workflow_dispatch` only for testing builds.

## Build Matrix

| Target | OS Runner | Artifact |
|--------|-----------|----------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `se-ai-translator-linux-x86_64.tar.gz` |
| `x86_64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-x86_64.tar.gz` |
| `aarch64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-aarch64.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `se-ai-translator-windows-x86_64.zip` |

## Release Flow

1. Update `version` in `Cargo.toml`, commit and push to `master`
2. Auto-tag workflow creates `v<version>` tag (from `Cargo.toml`)
3. Re-push the tag manually (see workaround below)
4. Release workflow fires on the tag push
5. Four parallel jobs build for each target (`fail-fast: false` — one failure doesn't cancel others)
6. A release job collects all artifacts and creates a GitHub Release

## Version Bumping

1. Update `version` in `Cargo.toml`
2. Commit and push/merge to `master`
3. Auto-tag creates the tag, but you must re-push it to trigger the release:

```bash
# The auto-tag workflow creates the tag, but GITHUB_TOKEN can't trigger release.
# Delete and re-push with your PAT to actually trigger it:
git tag -d v<x.y.z>
git push origin :refs/tags/v<x.y.z>
git tag v<x.y.z>
git push origin v<x.y.z>
```

The release workflow verifies the git tag matches `Cargo.toml` version — a mismatch fails the build.

> **Future fix:** Configure a Personal Access Token (PAT) with `contents:write` as a repo secret and use it in the auto-tag workflow's push step. This allows auto-tag to trigger release directly, eliminating the manual re-push.

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

## Cross-Platform Gotchas

- **Bundled assets:** Font files and images must be embedded via `include_bytes!("../assets/...")` — never use absolute system paths like `/usr/share/fonts/` (won't exist on macOS/Windows CI).
- **Linux system deps:** eframe needs GTK, XCB, XKB, and SSL dev libraries. The workflow installs them via `apt-get`.
- **Windows binary:** Packaged as `.zip` (via 7z); all others as `.tar.gz`.
- **`fail-fast: false`:** Set so one platform build failure doesn't cancel the others.
