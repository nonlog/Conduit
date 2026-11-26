# Conduit

English | [简体中文](README.zh-CN.md)

**A lightweight, end-to-end encrypted Android ↔ Windows companion** for clipboard sync, notifications, file transfers, and recent captures — designed to stay fast and quiet in the background.

---

## Why Conduit?

Microsoft Phone Link works, until it doesn't: over days of use, its native transport leaks threads and memory until the phone starts lagging, heating up, and draining battery.

Conduit takes the opposite approach. Instead of trying to do everything, it focuses strictly on the few things you actually use every day — built on a simple, event-driven background that stays just as light after weeks of uptime as it was on day one:

- **Near-zero idle overhead on Android** — No polling, no periodic network scans, no long wake locks, and no connection churn when switching networks.
- **Lightweight native Windows footprint** — A small Rust background daemon paired with an on-demand native desktop UI (`conduit-rune`), keeping memory, handles, and threads strictly bounded.
- **One clean connection lifecycle** — Direct LAN when possible, encrypted relay when away, with no ICE/STUN/TURN machinery.

> **Out of scope by design:** Telephony, SMS, screen mirroring, remote mouse/keyboard input, media controls, and browsable remote filesystems.

---

## Features

- **Two-way Clipboard Sync** — Automatically syncs text and images between your phone and PC. Supports seamless background reading on rooted Android via LSPosed, or an event-driven Accessibility fallback on stock Android.
- **Notifications → Windows Toasts** — Mirror Android notifications as native Windows toasts, complete with live updates, dismissal sync, notification actions, and inline replies.
- **Two-way File Transfer** — Send files from Android via the system Share sheet (including Direct Share targets for each paired PC), or from Windows via File Explorer and the native Windows Share Target.
- **Photo & Screenshot Capture Toasts** — Newly taken photos and screenshots on your phone pop up as Windows notifications, with optional one-click handoff to Snipping Tool.
- **Webpage Sharing** — Send links between phone and PC in one tap.
- **Multi-PC Support** — Pair your phone with multiple Windows computers and manage them individually from **Settings → Devices**.
- **End-to-End Encrypted Transport** — Prefers direct LAN connections via mDNS (`TCP 41112`) and falls back seamlessly to a blind TCP relay (`TCP 41113`) on cellular or foreign networks. All traffic is encrypted end-to-end with Noise (`Noise_XX_25519_ChaChaPoly_BLAKE2s`), so the relay only forwards opaque bytes and cannot read your data.

---

## Getting Started

### Requirements

- **Android:** Android 10 or newer (API 29+)
- **Windows:** Windows 10 / 11 (x64)

### 1. Install on Windows

Download the latest Windows package from [GitHub Releases](https://github.com/nonlog/Conduit/releases/latest) and extract it, or install via [Scoop](https://scoop.sh) if you use the [`nonlog/scoop-www`](https://github.com/nonlog/scoop-www) bucket:

```powershell
scoop bucket add www https://github.com/nonlog/scoop-www
scoop install www/conduit
```

### 2. Install on Android

Download and install the signed APK from [GitHub Releases](https://github.com/nonlog/Conduit/releases/latest).

### 3. Pair Your Devices

1. Launch **Conduit** on both Windows and Android.
2. On Windows, start pairing to display a 6-digit code and QR code.
3. On Android, scan the QR code or enter the 6-digit code, then verify the security fingerprint matches on both screens.

### 4. Enable Background Clipboard Sync on Android

Android 10+ restricts background apps from reading the clipboard. Conduit supports two zero-polling modes:

- **Rooted devices (LSPosed — recommended):** Enable the Conduit module in LSPosed for **System Framework** and **Conduit**, then reboot. Conduit will use the low-overhead hook automatically.
- **Stock Android (no root):** Enable **Conduit clipboard sync** in Android Accessibility settings when prompted. It only activates while a desktop is connected and never polls in the background.

---

## Project Structure

| Directory | Stack | Role |
|---|---|---|
| `android/` | Kotlin, Jetpack Compose | Android app, notification listener, and optional LSPosed clipboard module |
| `windows/conduit-daemon/` | Rust | Lightweight resident Windows background service |
| `windows/conduit-rune/` | Rust | On-demand native Windows desktop UI |
| `windows/conduit-share-target/` | Rust | Native Windows Share Target host |
| `relay/` | Rust | Stateless blind TCP rendezvous relay for off-LAN connections |
| `proto/` | Protocol Buffers | Shared wire contract (single source of truth for all platforms) |

---

## Documentation

For contributors and deeper technical details:

- [Architecture Overview](docs/architecture.md) — Transport, Noise XX handshake, framing, and security model
- [Development Guide](docs/development.md) — Building, testing, GitHub Actions CI, and installing dev builds
- [Design Decisions](docs/decisions.md) — Rationale behind key protocol and platform trade-offs
- [Roadmap & TODO](docs/TODO.md) — Open tasks and long-running validation checklist

Formal release binaries for Android, Windows, and the Relay are built reproducibly via GitHub Actions and published with `SHA256SUMS.txt` on [GitHub Releases](https://github.com/nonlog/Conduit/releases).

---

## Provenance

Conduit is a clean-room implementation. [KDE Connect](https://kdeconnect.kde.org/) and [Sefirah](https://github.com/shrimqy/Sefirah) were studied for protocol and architecture ideas, and no code was copied from either project.
