# SE5 AI Translator Plugin

SubtitleEdit 5 plugin that translates subtitles via the AI OpenSubtitles API.

## What It Is

A Rust binary that SubtitleEdit 5 launches as a plugin. SE5 writes a `request.json` (subtitle content + metadata), passes the path as CLI arg. Plugin shows an egui GUI, calls the OpenSubtitles translation API, writes a `response.json` with the translated subtitle, exits.

## SE5 Plugin Contract

- SE5 passes `request.json` path as first CLI argument
- Plugin reads request, does work, writes `response.json` to `responseFilePath` from request
- Exit code 0 = success; non-zero or missing response = error
- Response `status`: `"ok"` (apply changes), `"cancelled"` (no-op), `"error"` (show message)
- Settings persist via `settings` field in response — SE5 hands it back on next run
- Docs: https://github.com/SubtitleEdit/subtitleedit/blob/main/docs/plugin.md

## Architecture

```
src/
  main.rs           # Entry: parse CLI arg → launch eframe window
  se_contract.rs    # SE5 JSON types (SeRequest, SeResponse, PluginSettings) + read/write helpers
  api.rs            # OpenSubtitles API client (login, translate, poll, fetch engines/languages)
  ui.rs             # egui app (TranslatorApp) — states: Setup → Ready → Translating → Done/Error
```

## API Endpoints (OpenSubtitles)

Base: `https://api.opensubtitles.com/api/v1`
Auth: dual headers `Api-Key: <key>` + `Authorization: Bearer <token>`

- `POST /login` — `{username, password}` → token
- `POST /ai/translate` — multipart: file + translate_from + translate_to + api + return_content=true
- `POST /ai/translation/{correlationId}` — poll status until COMPLETED
- `POST /ai/info/translation_apis` — list available engines
- `POST /ai/info/translation_languages` — list languages (optional `{api}` filter)

## Key Design Decisions

- Source language defaults to auto-detect (API handles it); user can override
- Target language always user-selected
- Always uses `subRip` format from SE5 request
- Translation runs on a background thread; UI polls via `Arc<Mutex<Option<ThreadResult>>>`
- Cancel support via `AtomicBool` flag checked during polling

## Build

```bash
cargo check          # Quick compile check
cargo build --release  # Release binary (LTO+strip, ~10MB, takes ~7min)
```

Binary output: `target/release/se-ai-translator`

## Cross-Compilation Targets

Not yet configured. Needed: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`.

## Testing

Create a test `request.json` matching the SE5 contract schema and run:
```bash
./target/release/se-ai-translator /path/to/test_request.json
```

## Files

- `plugin.json` — SE5 manifest (must ship alongside binary in `Plugins/AI Translate (OpenSubtitles)/`)
- `icons/icon.png` — plugin icon (not yet created)
