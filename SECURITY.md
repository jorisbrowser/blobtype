# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Use GitHub's private
vulnerability reporting ("Security" tab → "Report a vulnerability") on this repository.
You will get a response as soon as possible; please include steps to reproduce and the
affected version.

## Scope and design notes

- Audio is sent only to the provider you configured, over HTTPS/WSS. Nothing is sent anywhere else
  and there is no telemetry.
- Audio is held in memory only and never written to disk. API keys and audio are never logged.
- API keys are read from environment variables or a `.env` file, never from `config.toml`.
- Text is injected with documented user-mode Win32 APIs (`SendInput`, clipboard). blobtype does not
  hook, inject into or read the memory of other processes.
