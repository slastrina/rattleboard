# Rattleboard

A split-flap desktop mirror for your [Vestaboard](https://www.vestaboard.com). Rattleboard sits in your macOS menu bar or Windows system tray. When the message on your board changes, a live replica flips into view (with the clatter, if you like) and then tucks away again.

Free and open source under the MIT license. Not affiliated with or endorsed by Vestaboard, Inc.

## Features

- **Live replica.** The flaps spin through the same drum order as the real hardware: blank, A–Z, 0–9, punctuation, then the colour chips. Flagship (6 × 22) and Note (3 × 15) boards are both supported, in black or white finish.
- **Flap sound.** An optional mechanical clatter, synthesized live while the flaps turn.
- **Two ways to show it.**
  - **Pop-up:** a toast that flips into view in the corner of your choice whenever the message changes, then hides after a countdown. The countdown pauses while your pointer is over the board. Drag it toward another corner and it snaps there.
  - **Pinned widget:** stays on screen wherever you drop it, either on the desktop below your windows or above them. Rattleboard remembers its spot.
- **Any size.** Hover over the board and drag its corner handle, or use Settings → Display → Size. The size is remembered.
- **Local API or Cloud API.** Read the board directly over your Wi-Fi, or through `cloud.vestaboard.com` from anywhere.
- **Built-in API server.** Rattleboard can act as a virtual Vestaboard. Your own apps and scripts send it messages using the same requests they'd send a real board. See [API server](#api-server).
- **Demo mode.** No board yet? It flips through sample messages so you can try everything.
- **Message history.** Shows recent changes, and **Flip again** replays any of them.

## Install

On macOS, install with [Homebrew](https://brew.sh):

```sh
brew install --cask slastrina/tap/rattleboard
```

Or download the latest installer from [Releases](https://github.com/slastrina/rattleboard/releases).

| Platform | File | First launch |
| --- | --- | --- |
| macOS 11+ (Apple Silicon and Intel) | `Rattleboard_x.y.z_universal.dmg` | Signed and notarized; just open it |
| Windows 10/11 | `Rattleboard_x.y.z_x64-setup.exe` or `.msi` | If SmartScreen appears, choose **More info → Run anyway** |

## Connecting your board

The first launch opens a short setup:

- **Local network** (recommended). Needs your board's IP address and a **Local API key**. Vestaboard issues the key when you enable the Local API for your board. Rattleboard calls `GET http://<board-ip>:7000/local-api/message`.
- **Cloud.** Needs a **Cloud API token**; see [Vestaboard's API docs](https://docs.vestaboard.com) for how to get one. Rattleboard calls `GET https://cloud.vestaboard.com/`.
- **Demo.** Needs nothing.
- **Virtual board.** Needs nothing. The board shows only what apps send to the [API server](#api-server).

You can change the source any time in **Settings → Connection**.

### Where your keys go

Keys and tokens are stored only in your operating system's credential store: the macOS Keychain or the Windows Credential Manager. They are never written to a file, logged, or sent anywhere except the Vestaboard endpoint you chose. Non-secret preferences (finish, size, corner and so on) live in `settings.json` in the app's config folder.

## API server

Turn it on in **Settings → API server**. Rattleboard then listens on a port; the default is `7000` on Windows and `7070` on macOS, because AirPlay Receiver already uses 7000 there. It accepts the request formats of all three Vestaboard APIs, so most existing Vestaboard clients work once you point them at Rattleboard's address and give them its key.

- By default only apps on this computer can connect. Turn on **Other devices** to accept requests from your network, for example from the Rattleboard phone apps.
- Rattleboard creates a random key the first time the server starts. It's kept in the OS credential store, and **New key** replaces it.
- Every request needs that key, in any one of these forms:
  - `X-Vestaboard-Local-Api-Key: <key>`
  - `X-Vestaboard-Read-Write-Key: <key>`
  - `X-Vestaboard-Token: <key>`
  - `Authorization: Bearer <key>`

| Request | Body | Response |
| --- | --- | --- |
| `GET /local-api/message` | — | The current layout: `[[0,8,5,…], …]` |
| `POST /local-api/message` | A layout array, `{"characters": [[…]]}` or `{"text": "…"}` | `201 {"status":"ok","id":"…"}` |
| `GET /` | — | `{"currentMessage": {"layout": "<JSON string>", "id": "…"}}` (Read/Write and Cloud API style) |
| `POST /` | Same bodies as above | `201 {"status":"ok","id":"…"}` |

- **Layouts** use Vestaboard character codes: 0 is blank, 1–26 are A–Z, 27–36 are 1–9 and 0, and 63–70 are the colour chips. A layout must be 6 rows × 22 columns (Flagship) or 3 × 15 (Note).
- **`text`** is upper-cased, word-wrapped and centred. Put `{63}` in the text to insert a raw character code.
- **Browsers:** CORS is open, so pages running in a browser can call the server too. They still need the key.

```sh
curl -X POST http://127.0.0.1:7070/local-api/message \
  -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY" \
  -d '{"text": "Build passed {66}"}'
```

While Rattleboard is mirroring a real board or the demo, a message posted to the API shows up right away. The mirrored board's next change replaces it. To use Rattleboard purely as a virtual board, set **Connection → Read from** to **Virtual board**.

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
| `src-tauri/src/vestaboard.rs` | Local and Cloud API clients, text layout, plus demo messages |
| `src-tauri/src/server.rs` | The built-in Vestaboard-compatible API server |
| `src-tauri/src/secrets.rs` | OS credential store |

### Releasing

Pushing a version tag builds the macOS (universal) and Windows installers in GitHub Actions and attaches them to a GitHub Release. The macOS build is signed and notarized when the Developer ID secrets listed in `.github/workflows/release.yml` are set on the repository.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## Credits

- Flap typeface: [Barlow Semi Condensed](https://github.com/jpt/barlow) by Jeremy Tribby, SIL Open Font License 1.1 (`src/fonts/OFL.txt`)
- Built with [Tauri](https://tauri.app)

## License

[MIT](LICENSE)
