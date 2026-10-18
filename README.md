# Rattleboard

A split-flap desktop mirror for your [Vestaboard](https://www.vestaboard.com). Rattleboard sits in your macOS menu bar or Windows system tray. When the message on your board changes, a live replica flips into view (with the clatter, if you like) and then tucks away again.

Free and open source under the MIT license. Not affiliated with or endorsed by Vestaboard, Inc.

<p align="center">
  <img src="docs/rattleboard-standard.gif" alt="Rattleboard replica of a Standard board (6 × 22) flipping between messages" width="760"><br>
  <sub>Standard (6 × 22)</sub>
</p>
<p align="center">
  <img src="docs/rattleboard-compact.gif" alt="Rattleboard replica of a Compact board (3 × 15) flipping between messages" width="560"><br>
  <sub>Compact (3 × 15)</sub>
</p>

## Features

- **Live replica.** The flaps spin through the same drum order as the real hardware: blank, A–Z, 0–9, punctuation, then the colour chips, in black or white finish.
- **Any message size.** By default the board shows each message at the size it was sent at, from 1 × 1 up to 12 × 40, and the pop-up or widget resizes to fit. Or fix it at Standard (6 × 22), Compact (3 × 15), Large (8 × 30) or a custom size. See [Board sizes](#board-sizes).
- **Transitions.** Messages can flip in all at once, or in a wave: by column, reverse column, from the edges to the centre, by row, diagonally or at random. See [Transitions](#transitions).
- **Flap sound.** An optional mechanical clatter, synthesized live while the flaps turn.
- **Three ways to show it.**
  - **Pop-up:** a toast that flips into view in the corner of your choice whenever the message changes, then hides after a countdown. The countdown pauses while your pointer is over the board. Drag it toward another corner and it snaps there.
  - **Pinned widget:** stays on screen wherever you drop it, either on the desktop below your windows or above them. Rattleboard remembers its spot.
  - **Full screen:** the board fills a whole display, for a spare computer used as a board. See [Use a spare computer as a board](#use-a-spare-computer-as-a-board).
- **Any size.** Hover over the board and drag its corner handle, or use Settings → Display → Size. The size is remembered.
- **Local API or Cloud API.** Read the board directly over your Wi-Fi, or through `cloud.vestaboard.com` from anywhere.
- **Built-in API server.** Rattleboard can act as a virtual Vestaboard. Your own apps and scripts send it messages using the same requests they'd send a real board. See [API server](#api-server).
- **Compose and Send.** Write a message as text or paint it tile by tile, preview it in flaps, then show it here or send it to a TV or computer running Rattleboard, or to a real Vestaboard: one board, a group of them, or all at once. See [Compose](#compose).
- **Sequences.** Play several messages in turn, each held for a while once its flaps settle, once or on a loop.
- **Pair a phone.** Show a QR code that the Rattleboard phone app scans to add this board.
- **Notification forwarding (Windows).** Put notifications from the apps you choose on a board. See [Notification forwarding](#notification-forwarding-windows).
- **Demo mode.** No board yet? It flips through sample messages so you can try everything.
- **Message history.** Shows recent changes. **Flip again** replays any of them here, **Send to** sends that exact layout to a saved board, and **Edit** opens it in Compose.

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

- **Local network** (recommended). Mirror another Rattleboard on your Wi-Fi: a TV, desktop or phone. Setup lists the Rattleboards it finds on your network; you need the other board's address (for example `192.168.1.20:7070`) and its API key, from its **Settings → API server**. Rattleboard calls `GET http://<address>/local-api/message`.
  - This also works with a Vestaboard's Local API: enter its IP address (port 7000 is assumed) and the **Local API key** Vestaboard issues when you enable the Local API for your board.
- **Cloud.** For a Vestaboard through the cloud. Needs a **Cloud API token**; see [Vestaboard's API docs](https://docs.vestaboard.com) for how to get one. Rattleboard calls `GET https://cloud.vestaboard.com/`.
- **Demo.** Needs nothing.
- **Virtual board.** Needs nothing. The board shows only what apps send to the [API server](#api-server).

You can change the source any time in **Settings → Connection**. A mirrored board takes the size of the board it reads.

## Use a spare computer as a board

An old Mac, a Windows laptop or a NUC behind a TV makes a good split-flap board. Full screen covers a whole display, menu bar, Dock and taskbar included, with the board centred on a solid background (near-black for the black finish, light for the white one) and scaled to fit with a comfortable margin. Every board size works; with **Match each message**, the tiles rescale whenever a message of another size arrives.

- **Turn it on** in **Settings → Display → Show as → Full screen**, with **Full screen** in the tray menu, or with F11 (and Control-Command-F on macOS). Esc leaves full screen, back to the pop-up or widget you had before.
- **Pick the display** under **Settings → Display → Display** when there's more than one. Rattleboard remembers it by name, and by position to tell two of the same model apart. If it's unplugged, the board moves to the main display, and it re-fits when the resolution changes.
- **Controls:** move the mouse or tap the screen for a small bar with the source and status, **Exit full screen**, **Replay**, **Pair a phone** and **Settings**. The bar and the pointer hide again after 3 seconds without movement.
- **Keep-awake:** while the board is full screen, the display doesn't sleep and the computer doesn't idle to sleep. It never hides, and the pop-up countdown doesn't apply. On macOS, `pmset -g assertions` lists it as "Rattleboard is showing the board full screen".
- **Running unattended:** in **Settings → Behavior**, turn on **Start at login** and **Full screen at launch**. The computer then starts straight into the board after a restart or a power cut. Without **Full screen at launch**, full screen lasts until Rattleboard quits.
- **Pairing phones:** **Pair a phone** shows the pairing QR code on the board's own screen (see [Pairing a phone](#pairing-a-phone)), so a phone can add the board without anyone touching the computer. It offers to turn on the API server and **Other devices** if they're off.
- The free desktop app shows a small "iokig.com" in the corner of the screen in full screen.

Everything else keeps working in full screen: the API server, sequences, Compose, Message History and the tray menu. Settings and Compose open above the board.

## Board sizes

**Settings → Display → Size** chooses how big the board is:

- **Match each message** (the default): every message shows at its own size, and the pop-up or widget resizes to fit before the flaps turn. A pop-up stays in its corner; a pinned widget keeps its top-left corner. Text that doesn't ask for a size, and the demo, use the size picked below it.
- **Fixed:** Standard (6 × 22), Compact (3 × 15), Large (8 × 30), or a custom size from 2 × 8 to 12 × 40. A message of another size is centred on the board, padded with blanks or cropped from the middle.

**Tile size** (or the board's corner handle) sets how big each tile is, so a Compact message makes a smaller board at the same tile size.

### Where your keys go

Keys and tokens are stored only in your operating system's credential store: the macOS Keychain or the Windows Credential Manager. That includes the key of every saved board in **Settings → Boards**, each under its own entry. They are never written to a file, logged, or sent anywhere except the board they belong to. Non-secret files live in the app's config folder: `settings.json` (finish, size, corner and so on), `boards.json` (saved boards' names, addresses and sizes), `groups.json` and `drafts.json` (Compose drafts); `history.json` and `recent.json` (messages sent from Compose) sit in the app's data folder.

## API server

Turn it on in **Settings → API server**. Rattleboard then listens on port `7070` by default. A real board uses 7000, but AirPlay receivers on Macs and many TVs already hold that port. It accepts the request formats of all three Vestaboard APIs, so most existing Vestaboard clients work once you point them at Rattleboard's address and give them its key.

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

- **Layouts** use Vestaboard character codes: 0 is blank, 1–26 are A–Z, 27–36 are 1–9 and 0, 37–62 are punctuation, 63–70 are the colour chips and 71 is a filled tile. A layout can be any rectangle from 1 × 1 to 12 rows × 40 columns. With **Match each message** it shows at its own size; a fixed-size board centres it.
- **`text`** is upper-cased, word-wrapped and centred. Put `{63}` in the text to insert a raw character code. It may ask for a size with `"rows"` (2–12) and `"cols"` (8–40): `{"text": "Hi", "rows": 3, "cols": 15}`. Rattleboard follows that with **Match each message**, and otherwise uses its own size.
- **Transitions:** the object forms also take `strategy`, `step_interval_ms` and `step_size`, as a Vestaboard's Local API does. See [Transitions](#transitions).
- **Browsers:** CORS is open, so pages running in a browser can call the server too. They still need the key.

```sh
curl -X POST http://127.0.0.1:7070/local-api/message \
  -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY" \
  -d '{"text": "Build passed {66}"}'
```

```sh
# Flip it in column by column, two columns every 80 ms
curl -X POST http://127.0.0.1:7070/local-api/message \
  -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY" \
  -d '{"text": "Doors open", "strategy": "column", "step_interval_ms": 80, "step_size": 2}'
```

While Rattleboard is mirroring a real board or the demo, a message posted to the API shows up right away. The mirrored board's next change replaces it. To use Rattleboard purely as a virtual board, set **Connection → Read from** to **Virtual board**.

### Rattleboard endpoints

Rattleboards also answer a few requests of their own under `/rattleboard/`. They take the same key.

| Request | Body | Response |
| --- | --- | --- |
| `GET /rattleboard/info` | — | `{"app":"rattleboard","protocol":2,"version":"0.2.0","platform":"macos","name":"…","board":{"rows":6,"cols":22},"adaptive":true,"features":["sequence","strategy"]}` |
| `POST /rattleboard/sequence` | `{"name":"…","loop":true,"frames":[…]}` (up to 256 KB) | `201 {"status":"ok","id":"seq-…","frames":2}` |
| `GET /rattleboard/sequence` | — | `{"running":true,"id":"seq-…","name":"Morning","frame":2,"frames":5,"loop":true}` or `{"running":false}` |
| `DELETE /rattleboard/sequence` | — | `204`; the current frame stays on the board |

A real Vestaboard answers `404` or `401` to these, which is how apps tell the two apart.

A sequence has 1–100 frames. Each frame is `characters` or `text` (with an optional size hint), as in a normal post, with `hold` (1–3600 seconds) and an optional transition. With **Match each message**, each frame can have its own size. A frame is held for `hold` seconds after its flaps finish, so it's never cut off mid-flip. Without `"loop": true`, the last frame stays up. Starting a sequence replaces the one playing; any normal message stops it.

```sh
curl -X POST http://127.0.0.1:7070/rattleboard/sequence \
  -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY" \
  -d '{"name": "Morning", "loop": true, "frames": [
        {"text": "Good morning", "hold": 8, "strategy": "edges-to-center"},
        {"text": "Coffee is ready {66}", "hold": 5}
      ]}'

curl http://127.0.0.1:7070/rattleboard/sequence -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY"
curl -X DELETE http://127.0.0.1:7070/rattleboard/sequence -H "X-Vestaboard-Local-Api-Key: $RATTLEBOARD_KEY"
```

The tray menu shows a playing sequence ("Playing ‘Morning’ 2/5") with a **Stop** button.

### Pairing a phone

**Settings → API server → Show pairing code** shows a QR code that the Rattleboard phone app (or the phone's camera) scans to add this board. It needs **Other devices** turned on, and offers to turn it on. The code holds this computer's name and network address, the board's size and the API key, so it's shown only when you ask and disappears after two minutes. **Copy link** copies the same `rattleboard://pair?…` link, which another Rattleboard desktop can paste under **Settings → Boards → Pair link**.

With **Other devices** on, Rattleboard also announces itself on your network (as `Rattleboard (<computer name>)`, an `_http._tcp` service) so other Rattleboards can find it.

## Transitions

| `strategy` | Flaps start |
| --- | --- |
| `classic` (default) | all together |
| `column` | left to right, a column per step |
| `reverse-column` | right to left |
| `edges-to-center` | from the outer columns in to the middle |
| `row` | top to bottom |
| `diagonal` | from the top-left corner, along diagonals |
| `random` | in a random order of tiles |

`step_size` (1–40, default 1) is how many columns, rows, diagonals or tiles start together, and `step_interval_ms` (1–5000, default 120) is the time between steps. An unknown strategy, or a value out of range, is a `400`.

## Compose

Open **Compose…** from the tray menu or **Settings → Boards**.

- **Text** lays out what you type exactly as a board lays out posted text, and previews it in flaps as you type. `{r}` `{o}` `{y}` `{g}` `{b}` `{v}` `{w}` `{k}` (or `{63}`–`{70}`) add colour chips.
- **Grid** lets you paint tiles: pick a letter, symbol or colour chip and click or drag across the board, or click a tile and type.
- **Transition** picks how the message flips in, and **Replay** shows it again.
- **Sequence:** **Add frame** turns the message into a list of frames. Each frame has its own hold and transition, and frames can be reordered, duplicated and deleted. **Preview** plays the sequence in Compose without sending anything.
- **Drafts** are saved in `drafts.json`, never with keys.
- **Recent** lists the last 50 messages sent from Compose. Each can be sent again, to the last board used or another, or edited. The tray menu also offers **Send this to <board>** for the board you sent to last.
- A layout is always sent exactly as drawn. If it was made for a board of another size, Compose says so; the receiving board centres it.

**Send to** picks **This board**, **All boards**, a group or one saved board, and the editor takes the board's size. For boards that match each message (this one, by default, and other Rattleboard desktops), **Message size** picks the size to send at; Compose remembers it.

- **This board** shows the message here, like a message sent to the API server.
- **Another Rattleboard** gets the message on its API, and a sequence is handed to it to play by itself.
- **A real Vestaboard** gets each message through its Local API or the Cloud API, at least 5 s apart on the Local API and 15 s apart on the Cloud, as Vestaboard's limits require; a message sent sooner waits its turn. For a sequence, this computer sends frame after frame.
- **A group, or all boards:** every board gets the message at once, each laid out for its own size: text is laid out per board, and a grid is centred on fixed-size boards of another size. Compose lists how each board took it (sent, queued, rate-limited or failed, with **Retry**), and one board failing never holds up the others. A sequence plays on all of them, with one status line and **Stop all**.

### Saved boards

**Settings → Boards** lists the boards you can send to. Add one in any of these ways:

- **Nearby:** Rattleboards that announce themselves on your network (TVs, other computers). Pick one and enter its key.
- **By address:** a Rattleboard's address and API key, or a Vestaboard's IP address and Local API key. Rattleboard asks `GET /rattleboard/info` to tell which it is.
- **Pair link:** paste a `rattleboard://pair?…` link from another Rattleboard. Rattleboard shows which board it is and asks before adding it.
- **Cloud:** a Vestaboard's Cloud API token.

Each board can be renamed or removed. Removing it deletes its key from the credential store.

**Groups** are named sets of boards, such as "Downstairs", to send to at once. A board can be in any number of groups. They're kept in `groups.json` (just names and board ids).

## Notification forwarding (Windows)

On Windows, **Settings → Notifications** puts notifications from the apps you choose on this board or a saved one.

- **Allow access…** asks Windows for permission to read notifications. You can change this later in Windows Settings → Privacy & security → Notifications.
- Apps show up in the list once they've sent a notification; tick the ones to forward. **Show on** can be this board, a saved board, a group or all boards.
- **Hide content** shows only which app it's from. **Quiet hours** pauses forwarding, for example overnight.
- **Go back after** puts the previous message back after a while, but only if the board still shows the notification.

macOS has no supported way for apps to read other apps' notifications, so this setting doesn't appear there.

## Building from source

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 18+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform (on Windows, WebView2 and the MSVC build tools).

```sh
npm install
npm run dev      # run with hot reload
npm run build    # produce installers in src-tauri/target/release/bundle
```

To try changes without touching your own settings and keys, run a debug build with `RATTLEBOARD_PROFILE=<name>`: it uses config and data folders of its own (`…rattleboard.<name>`) and keeps its keys in a `secrets.json` there instead of the credential store. Release builds ignore it.

The UI is plain HTML, CSS and JavaScript in `src/`, with no bundler. The Rust side, in `src-tauri/src/`, handles polling, the keychain, the tray icon and window placement.

| Path | What it is |
| --- | --- |
| `src/shared/split-flap.js` | The `<split-flap-board>` web component: flap animation and sound |
| `src/board.*` | The floating board window |
| `src/tray.*` | The menu-bar / tray popover |
| `src/panel.*` | Settings, Message History and first-run setup |
| `src/compose.js` | Compose: the editor, sequences and drafts |
| `src-tauri/src/vestaboard.rs` | Local and Cloud API clients, text layout, transitions, plus demo messages |
| `src-tauri/src/server.rs` | The built-in Vestaboard-compatible API server and the `/rattleboard/` endpoints |
| `src-tauri/src/sequence.rs` | Sequence validation |
| `src-tauri/src/boards.rs` | Saved boards, pair links, network discovery |
| `src-tauri/src/send.rs` | Groups, and sending to several boards at once |
| `src-tauri/src/notify.rs` | Notification forwarding (Windows) |
| `src-tauri/src/fullscreen.rs` | Full screen: picking the display, keep-awake, hiding the pointer |
| `src-tauri/src/secrets.rs` | OS credential store |

### Releasing

Pushing a version tag builds the macOS (universal) and Windows installers in GitHub Actions and attaches them to a GitHub Release. The macOS build is signed and notarized when the Developer ID secrets listed in `.github/workflows/release.yml` are set on the repository.

```sh
git tag v0.2.0 && git push origin v0.2.0
```

## Credits

- Flap typeface: [Barlow Semi Condensed](https://github.com/jpt/barlow) by Jeremy Tribby, SIL Open Font License 1.1 (`src/fonts/OFL.txt`)
- Built with [Tauri](https://tauri.app)

## License

[MIT](LICENSE)
