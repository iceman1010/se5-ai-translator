# SE5 AI Translator Plugin

A [SubtitleEdit 5](https://github.com/SubtitleEdit/subtitleedit) plugin that translates subtitles using the [AI OpenSubtitles](https://ai.opensubtitles.com) translation API. It supports automatic language detection, multiple AI engines, and over 100 languages.

Powered by [OpenSubtitles](https://www.opensubtitles.com) — the world's largest subtitle platform.

## Features

- Translate subtitles to 100+ languages via AI
- Automatic source language detection
- Multiple AI translation engines to choose from
- Translation runs in the background — UI stays responsive
- Cancel a translation in progress
- Built-in self-update mechanism
- Cross-platform: Windows, macOS, Linux

## Requirements

- [SubtitleEdit 5](https://github.com/SubtitleEdit/subtitleedit/releases) (5.0.0 or later)
- An [OpenSubtitles](https://www.opensubtitles.com) account with API credits
  (credits can be purchased at [ai.opensubtitles.com/credits](https://ai.opensubtitles.com/credits) and never expire)

## Installation

1. Download the latest release for your platform from the [Releases](https://github.com/iceman1010/se5-ai-translator/releases) page
2. Extract the archive — it contains a folder named `AI Translate (OpenSubtitles)` with:
   - `plugin.json` (manifest)
   - `se-ai-translator` (or `se-ai-translator.exe` on Windows)
   - `icon.png`
3. Copy the entire folder into SubtitleEdit's `Plugins` directory:

   | Platform | Plugins directory |
   |----------|-------------------|
   | **Windows** (installed) | `%APPDATA%\Subtitle Edit\Plugins\` |
   | **Windows** (portable) | `<SubtitleEdit folder>\Plugins\` |
   | **Linux** | `~/.local/share/SubtitleEdit/Plugins\` |
   | **macOS** | `~/Library/Application Support/SubtitleEdit/Plugins/` |

   You can also find the Plugins folder from SubtitleEdit: go to **Plugins → Manage plugins...** and click **Open plugins folder**.
4. Restart SubtitleEdit if it was running

The folder structure should look like:

```
Plugins/
  AI Translate (OpenSubtitles)/
    plugin.json
    se-ai-translator
    icon.png
```

## Usage

1. Open a subtitle file in SubtitleEdit 5
2. Go to **Plugins → AI Translate (OpenSubtitles)** (or press **Ctrl+Shift+T**)
3. Log in with your OpenSubtitles account
4. Select target language and AI engine
5. Click **Translate**

Source language defaults to auto-detect. You can override it in the UI if needed.

## Building from Source

Requires [Rust](https://rustup.rs/) installed.

```bash
cargo build --release
```

The binary is output at `target/release/se-ai-translator`. On Linux, install these dependencies first:

```
libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
libspeechd-dev libxkbcommon-dev libssl-dev pkg-config
```

## License

[MIT](LICENSE)
