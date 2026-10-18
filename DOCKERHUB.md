# Rattleboard Web

A split-flap message board for any browser: a wall tablet, a TV's browser, a Raspberry Pi kiosk or a Home Assistant dashboard. It has the same API as every Rattleboard and runs Sources (clock, countdowns, weather, feeds, calendars, quote of the day) on the server.

**Source, docs and issues:** https://github.com/slastrina/rattleboard
**Website:** https://iokig.com/rattleboard

## Run it

```sh
docker run -d --name rattleboard --restart unless-stopped \
  -p 7070:7070 -v rattleboard:/data slastrina/rattleboard-web
```

Open `http://<server>:7070/` on any screen and tap once for full screen. Images for amd64, arm64 and arm/v7 (Raspberry Pi).

Get the API key, then send a message:

```sh
docker exec rattleboard rattleboard-web key
curl -X POST http://<server>:7070/local-api/message \
  -H "X-Vestaboard-Local-Api-Key: <key>" -d '{"text": "Hello {66}"}'
```

It also accepts the Vestaboard Local API request format, so existing scripts work unchanged.

## Settings

Environment variables: `RATTLEBOARD_KEY`, `RATTLEBOARD_NAME`, `RATTLEBOARD_SIZE` (standard, compact, large, custom), `RATTLEBOARD_FINISH` (black, white), `RATTLEBOARD_VIEWERS` (open, token), `TZ`. The full table, the Home Assistant recipe, pairing and the Raspberry Pi guide are in the [README](https://github.com/slastrina/rattleboard#rattleboard-web-docker).

Also published as `ghcr.io/slastrina/rattleboard-web`. MIT licensed.

Vestaboard is a trademark of Vestaboard, Inc. Rattleboard is not affiliated with or endorsed by Vestaboard.
