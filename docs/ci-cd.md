# CI/CD — Release Pipeline

Automated cross-platform builds via GitHub Actions.

## Workflow

### Release (`.github/workflows/release.yml`)

A single unified workflow that handles everything on push to `master`:

1. **check-version** — Reads `version` from `Cargo.toml`. If a `v<version>` tag already exists, the workflow exits (no-op). Otherwise, proceeds to build + release.
2. **build** — Builds all four targets in parallel (`fail-fast: false`), packages each as an archive.
3. **release** — Creates the git tag, then creates a GitHub Release with all artifacts attached.

Also supports `workflow_dispatch` for manual triggering (build only — the release step skips if the tag already exists or can't be pushed).

> **Why not two separate workflows?** Previously, an `auto-tag` workflow created a tag and a separate `release` workflow triggered on that tag. But tags pushed by `GITHUB_TOKEN` [cannot trigger other workflows](https://docs.github.com/en/actions/using-workflows/triggering-a-workflow#triggering-a-workflow-from-a-workflow). Merging into one workflow eliminates the chaining problem entirely — no PAT needed.

## Build Matrix

| Target | OS Runner | Artifact |
|--------|-----------|----------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `se-ai-translator-linux-x86_64.tar.gz` |
| `x86_64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-x86_64.tar.gz` |
| `aarch64-apple-darwin` | `macos-latest` | `se-ai-translator-macos-aarch64.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `se-ai-translator-windows-x86_64.zip` |

## Release Flow

1. Update `version` in `Cargo.toml`
2. Commit and push/merge to `master`
3. The workflow automatically: builds all 4 targets → creates tag → publishes release

No manual steps required.

## Self-Update

The plugin has a built-in self-update mechanism (Settings tab → "Check for Updates"):

1. Fetches latest release info from GitHub API
2. Compares versions
3. Downloads the platform-appropriate artifact
4. Extracts and replaces the running binary via `self-replace`
5. Next SE5 launch uses the updated binary

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
