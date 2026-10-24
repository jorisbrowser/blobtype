# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.3] - 2026-10-24

### Changed
- License changed from MIT to GPL-3.0: blobtype stays free and open source, and so must every
  copy or modified version that is shared or sold.

## [0.1.2] - 2026-10-24

### Added
- Optional "Mute Discord while recording": blobtype presses a key (F13-F24, bound to Discord's
  "Toggle Mute") when a recording starts and again when it stops. The settings window can send the
  key so Discord can record the keybind.
- "Insert last dictation again" shortcut (default Ctrl+Shift+Space): types the last transcript once
  more, e.g. when it went to the wrong window. Kept in memory only and always typed, so the clipboard
  is never touched.
- The blob shows the available shortcuts next to it, and briefly confirms "Inserted" with the
  "insert again" shortcut.

### Changed
- The waveform fills the blob.

## [0.1.1] - 2026-10-24

### Changed
- The shortcut is paused while a fullscreen app (most games) has focus, so the key combination
  reaches the game instead of starting a recording. Turn this off with
  `pause_in_fullscreen = false` in `config.toml`.

## [0.1.0] - 2026-10-23

First release.

- Global shortcut (default `Ctrl+Space`) starts and stops dictation; `Esc` cancels.
- Click-through overlay with a live waveform that never steals focus.
- Speech services: Groq, OpenAI, Deepgram, AssemblyAI, ElevenLabs, Mistral, Google Gemini and
  OpenAI-compatible self-hosted servers. Live typing with OpenAI Realtime and Deepgram.
- Text is typed via `SendInput` (Unicode) or pasted from the clipboard.
- First-run setup: choose a service, paste a key, test it, try it.
- Settings window for everything: service, key, shortcut, language, microphone, live typing,
  insert method, sounds, start with Windows.
