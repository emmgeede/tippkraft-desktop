# Security

A text expander has to read your keyboard. That is a lot of trust to ask for, so this page says exactly what the tippkraft desktop client does with it. Everything here can be checked in the source code of this repository.

## What it reads

- `tippkraft run` opens your keyboards through evdev (that is why your user needs to be in the `input` group). It takes them over exclusively and passes every key on unchanged through a virtual keyboard (`/dev/uinput`). Only the trigger key of a matching abbreviation is held back. See the comment at the top of [`desktop/src/main.rs`](desktop/src/main.rs).
- Mice, touchpads and touch screens are never taken over, only plain keyboards.
- To recognize abbreviations it keeps the last **64 characters** in memory ([`PUFFER_MAX` in `desktop/src/engine.rs`](desktop/src/engine.rs)). The buffer is cleared on clicks, navigation keys and shortcuts with a modifier. Its debug output only shows the length, never the content.
- In programs on your exclusion list (for example password managers like 1Password or KeePassXC) it does not trigger.

## What it stores

| File | Content | Permissions |
|---|---|---|
| `~/.config/tippkraft/zugang.json` | server URL and device token | `0600`, folder `0700` |
| `~/.local/share/tippkraft/stand.json` | your snippets and exclusion list | `0600`, folder `0700` |
| `$XDG_RUNTIME_DIR/tippkraft.sock` | control socket for `pause`, `status` and the tray | `0600` |

What you type is never written to disk, never logged and never sent anywhere.

## What it sends

The client talks only to the server you connected, by default `https://tippkraft.com`:

- `GET /api/sync/snapshot` and `GET /api/sync/stream` fetch your snippets, authenticated with the device token.
- `POST /api/sync/token` once when you connect a device. Connecting uses a browser login with a callback to `127.0.0.1` and PKCE (RFC 8252, RFC 7636), so your password never passes through the client.

No telemetry, no analytics, no crash reports. Updates are not installed automatically: the server tells the client which version is current, and the tray shows a hint. You update by running the install script again.

## Install script and releases

[`install.sh`](install.sh) never uses sudo. It downloads the release binary from this repository's GitHub releases and checks its SHA-256 checksum before running it. Release binaries are built by GitHub Actions from the tagged source ([`.github/workflows/release.yml`](.github/workflows/release.yml)).

## Server side

The web app and server are not open source. Your snippets are stored on the tippkraft server and only used to sync them to your devices. Where it runs and which processors are involved is listed in the [privacy policy](https://tippkraft.com/privacy).

## Reporting a vulnerability

Please do not open a public issue. Use [GitHub's private vulnerability reporting](https://github.com/emmgeede/tippkraft-desktop/security/advisories/new) or write to impressum@tippkraft.com. You will get an answer within a few days. Please give us a reasonable time to fix the problem before you publish it.

Supported version: only the latest release.
