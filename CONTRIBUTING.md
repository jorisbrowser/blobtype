# Contributing

Thanks for helping! Issues and pull requests are welcome.

## Layout

| Path | What |
| --- | --- |
| `crates/blobtype-core` | Config, audio helpers, STT providers, live transcription. No GUI, fully unit-tested, builds on any OS. |
| `crates/blobtype-input` | Text injection (`SendInput`, clipboard). Windows-only code, tiny stubs elsewhere. |
| `src-tauri` | Tauri 2 shell: tray, hotkeys, overlay window, mic capture, state machine. |
| `ui` | Plain HTML/CSS/JS for the blob overlay (no bundler). |

Keep logic in `blobtype-core` whenever possible so it can be tested without a GUI.

## Before opening a PR

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

On Linux the Tauri crate needs WebKitGTK dev packages to compile; `cargo test -p blobtype-core -p blobtype-input`
works everywhere.

## Adding a provider

1. Add `crates/blobtype-core/src/providers/<name>.rs` implementing `Provider` (`name()`,
   `async transcribe(wav)`). Keep request building in a pure function so it can be unit-tested; see
   `deepgram.rs`. For OpenAI-style multipart APIs reuse `openai_style.rs`.
2. Register it in `providers/mod.rs` and add its key env var in `config.rs::default_key_env`.
3. Add it to `settings.rs::providers()` so it shows up in the settings window.

Verify the request format against the provider's current documentation and add tests for request
building and response parsing.

## Conventions

- No API keys, audio or personal data in code, tests, logs or fixtures.
- Short, user-readable error messages (they are shown in the overlay).
- Update `CHANGELOG.md` for user-visible changes.
