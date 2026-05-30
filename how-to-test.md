# How to Test the Plugin

## 1. Local Test (Without SubtitleEdit)

Test the plugin directly with a fake `request.json` to verify the GUI and API integration.

### Create a test request

```bash
mkdir -p /tmp/se-test
cat > /tmp/se-test/request.json << 'EOF'
{
  "apiVersion": 1,
  "requestType": "run",
  "responseFilePath": "/tmp/se-test/response.json",
  "tempDirectory": "/tmp/se-test",
  "subtitle": {
    "format": "SubRip",
    "fileName": "test.srt",
    "native": "1\n00:00:01,000 --> 00:00:03,000\nHello world\n\n2\n00:00:04,000 --> 00:00:06,500\nThis is a test subtitle\n\n3\n00:00:07,000 --> 00:00:09,000\nGoodbye\n",
    "subRip": "1\n00:00:01,000 --> 00:00:03,000\nHello world\n\n2\n00:00:04,000 --> 00:00:06,500\nThis is a test subtitle\n\n3\n00:00:07,000 --> 00:00:09,000\nGoodbye\n"
  },
  "selectedIndices": [],
  "videoFileName": null,
  "frameRate": 23.976,
  "videoDurationSeconds": 10.0,
  "videoWidth": 1920,
  "videoHeight": 1080,
  "uiLanguage": "English",
  "theme": "Dark",
  "seVersion": "5.0.0",
  "settings": null,
  "settingsVersion": null
}
EOF
```

### Run the plugin

```bash
./target/release/se-ai-translator /tmp/se-test/request.json
```

### What to expect

1. A window opens with "Enter your API key and login credentials"
2. Enter your OpenSubtitles API key, username, password → click Login
3. Translation dialog appears with engine/language pickers
4. Click Translate → progress bar → "Translation complete!"
5. Click OK → window closes
6. Check the result:
```bash
cat /tmp/se-test/response.json
```

You should see `status: "ok"` with the translated subtitle in `subtitle.native`.

### Test cancel behaviour

Run again, click Cancel during translation. The response should show `status: "cancelled"`.

### Test with saved settings

Run a second time. The plugin should skip the login screen and go straight to the translation dialog (credentials are persisted in the response `settings` field).

---

## 2. Install Into SubtitleEdit 5

### Find the SE5 data folder

- **Windows**: `%APPDATA%\SubtitleEdit\` or next to `SubtitleEdit.exe` in portable installs
- **Linux**: `~/.config/SubtitleEdit/` or next to the binary
- **macOS**: `~/Library/Application Support/SubtitleEdit/`

Look for `Settings.xml` or `Plugins/` folder there.

### Install the plugin

```bash
# Create the plugin folder
mkdir -p "<SE5_DATA_FOLDER>/Plugins/AI Translate (OpenSubtitles)"

# Copy the binary
cp target/release/se-ai-translator "<SE5_DATA_FOLDER>/Plugins/AI Translate (OpenSubtitles)/"

# Copy the manifest
cp plugin.json "<SE5_DATA_FOLDER>/Plugins/AI Translate (OpenSubtitles)/"

# Copy icon (once you have one)
cp icons/icon.png "<SE5_DATA_FOLDER>/Plugins/AI Translate (OpenSubtitles)/"
```

The final layout should look like:
```
Plugins/
  AI Translate (OpenSubtitles)/
    plugin.json
    se-ai-translator
    icon.png
```

### Verify in SubtitleEdit

1. Open SubtitleEdit 5
2. Look for the **Plugins** menu in the menu bar
3. If missing: enable it via **Options → Settings → Appearance → Show Plugins menu**
4. Click **Plugins → AI Translate (OpenSubtitles)**
5. The plugin window should open

### Using the Plugin Manager

Alternatively, use SE5's built-in manager:

1. **Plugins → Manage plugins...**
2. See the plugin listed with enable/disable/remove options
3. **Get plugins online...** can install from the online index (requires publishing to the SE5 plugin index first)

---

## 3. Build for Windows (Cross-Compilation)

If you need to test on Windows:

```bash
rustup target add x86_64-pc-windows-msvc
# Requires a Windows linker — easiest to build natively on Windows, or use cross:
cargo install cross
cross build --release --target x86_64-pc-windows-msvc
```

Output: `target/x86_64-pc-windows-msvc/release/se-ai-translator.exe`

---

## 4. Debugging Tips

- **No window appears**: Check stderr output — `eprintln!` messages go to the terminal
- **"Invalid request JSON"**: Validate your test `request.json` with `jq . < request.json`
- **API errors**: The plugin shows error messages in the GUI and writes them to `response.json`
- **SE5 says "plugin failed"**: Check that `plugin.json` `executables.linux` matches the actual binary name, and the binary is executable (`chmod +x`)
- **Font not found**: The plugin embeds DejaVuSans from `/usr/share/fonts/truetype/dejavu/`. If missing on your system, install `fonts-dejavu-core` or change the path in `ui.rs`
