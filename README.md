# Zeron

Control your coding agents (Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, Antigravity) locally by default, with optional multi-device sync.

*English | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

> This is ReinaMacCredy's fork of [zeronsh/zeron](https://github.com/zeronsh/zeron) (MIT), kept in sync with upstream and carrying its own features. Its builds update from this repository's releases, not from upstream.

![Zeron desktop app](docs/media/readme/app-screenshot.jpg)

## Desktop app

Download the latest release for your platform from [GitHub Releases](https://github.com/ReinaMacCredy/zeron/releases/latest):

- **macOS** — `zeron-<version>-macos-arm64.dmg`
- **Windows** — `zeron-<version>-windows-x86_64-setup.exe`
- **Linux** — `zeron-<version>-linux-<arch>.tar.gz`, then run its `install.sh`

No account or network connection is needed; sessions stay on your device. The app checks for updates and offers them in the sidebar; set `ZERON_AUTO_UPDATE=1` to download them in the background and install on quit.

Fork builds are not Developer ID signed or notarized. On macOS, the first launch is blocked by Gatekeeper: right-click Zeron in Applications and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/Zeron.app`. Each update carries a new ad-hoc signature, so macOS asks for Screen Recording and Accessibility again after updating. On Windows, SmartScreen warns about an unknown publisher: choose **More info**, then **Run anyway**.

## Headless (CLI)

For servers and other machines without a display, such as a VPS that keeps agents running after you close your laptop. Linux only: download `zeron-<version>-linux-<arch>.tar.gz` from [GitHub Releases](https://github.com/ReinaMacCredy/zeron/releases/latest), then run:

```bash
tar -xzf zeron-<version>-linux-<arch>.tar.gz
zeron-<version>-linux-<arch>/install.sh
zeron daemon install
zeron status
```

`zeron daemon install` starts the engine as a background service that survives reboots.

```bash
zeron status      # local/synced mode and engine status
zeron update      # update to the latest release
zeron daemon start|stop|restart|status
```

## Multi-device sync (optional)

Sign in to start an agent on one device and follow or drive it from another:

```bash
zeron daemon stop
zeron login        # or: zeron logout to return to local-only
zeron daemon start
```

Devices signed in to the same account can read and write each other's workspace files, so only sign in devices you trust. Existing local sessions are never uploaded.

## Sponsors

Thank you to [The Context Company](https://www.thecontextcompany.com/) for sponsoring Zeron. You can help fund Zeron's development too by [becoming a sponsor on GitHub](https://github.com/sponsors/zeronsh).

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/zeronsh/zeron) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
