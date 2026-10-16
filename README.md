# Rattleboard

A split-flap desktop mirror for your [Vestaboard](https://www.vestaboard.com). Rattleboard sits in your macOS menu bar or Windows system tray. When the message on your board changes, a live replica flips into view (with the clatter, if you like) and then tucks away again.

Free and open source under the MIT license. Not affiliated with or endorsed by Vestaboard, Inc.

## Features

- **Live replica.** The flaps spin through the same drum order as the real hardware: blank, A–Z, 0–9, punctuation, then the colour chips. Flagship (6 × 22) and Note (3 × 15) boards are both supported, in black or white finish.
- **Flap sound.** An optional mechanical clatter, synthesized live while the flaps turn.
- **Pops up in a corner of your choice.** Drag the board anywhere; release it near a corner and it snaps there. It hides after a countdown, and the countdown pauses while your pointer is over the board.
- **Pin as widget.** Keeps the board on the desktop, below your other windows.
- **Local API or Cloud API.** Read the board directly over your Wi-Fi, or through `cloud.vestaboard.com` from anywhere.
- **Demo mode.** No board yet? It flips through sample messages so you can try everything.
- **Message history.** Shows recent changes, and **Flip again** replays any of them.

## Install

Download the latest installer from [Releases](https://github.com/slastrina/rattleboard/releases).

| Platform | File | First launch |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `Rattleboard_x.y.z_universal.dmg` | The app isn't notarized. Right-click Rattleboard in Applications and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/Rattleboard.app` |
| Windows 10/11 | `Rattleboard_x.y.z_x64-setup.exe` or `.msi` | If SmartScreen appears, choose **More info → Run anyway** |

## Connecting your board

The first launch opens a short setup:

- **Local network** (recommended). Needs your board's IP address and a **Local API key**. Vestaboard issues the key when you enable the Local API for your board. Rattleboard calls `GET http://<board-ip>:7000/local-api/message`.
- **Cloud.** Needs a **Cloud API token**; see [Vestaboard's API docs](https://docs.vestaboard.com) for how to get one. Rattleboard calls `GET https://cloud.vestaboard.com/`.
- **Demo.** Needs nothing.

You can change the source any time in **Settings → Connection**.

### Where your keys go

Keys and tokens are stored only in your operating system's credential store: the macOS Keychain or the Windows Credential Manager. They are never written to a file, logged, or sent anywhere except the Vestaboard endpoint you chose. Non-secret preferences (finish, size, corner and so on) live in `settings.json` in the app's config folder.

## Building from source

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 18+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform (on Windows, WebView2 and the MSVC build tools).

```sh
npm install
npm run dev      # run with hot reload
npm run build    # produce installers in src-tauri/target/release/bundle
```

The UI is plain HTML, CSS and JavaScript in `src/`, with no bundler. The Rust side, in `src-tauri/src/`, handles polling, the keychain, the tray icon and window placement.

| Path | What it is |
| --- | --- |
| `src/shared/split-flap.js` | The `<split-flap-board>` web component: flap animation and sound |
| `src/board.*` | The floating board window |
| `src/tray.*` | The menu-bar / tray popover |
| `src/panel.*` | Settings, Message History and first-run setup |
| `src-tauri/src/vestaboard.rs` | Local and Cloud API clients, plus demo messages |
| `src-tauri/src/secrets.rs` | OS credential store |

### Releasing

Pushing a version tag builds the macOS (universal) and Windows installers in GitHub Actions and attaches them to a GitHub Release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## Credits

- Flap typeface: [Barlow Semi Condensed](https://github.com/jpt/barlow) by Jeremy Tribby, SIL Open Font License 1.1 (`src/fonts/OFL.txt`)
- Built with [Tauri](https://tauri.app)

## License

[MIT](LICENSE)
