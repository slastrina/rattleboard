---
name: rattleboard
description: Put short notices on the user's Rattleboard split-flap board with the rattleboard MCP tools (send_message, show_status, play_sequence and others). Use it when a long task finishes, something fails, or you need the user's input and they may not be watching the screen. Also use it when the user asks you to put something on their board.
---

# Rattleboard

Rattleboard is a split-flap message board on the user's desk or wall. They see it from across the room, so a message there is an interruption. Make each one count.

## When to post

- **A long task finished.** Builds, test runs, migrations, refactors or research that took minutes. Say what happened: `TESTS PASSED`, `DEPLOY DONE`.
- **Something failed** and needs the user: `BUILD FAILED` and the module or step, short.
- **You need input** and are blocked, for example waiting on a decision, a permission or a credential: `NEED YOUR INPUT`.
- **The user asked** you to put something on the board.

Don't post routine progress, every step of a plan, or anything the user is clearly watching happen. Post **a few messages per session at most**, and not two in a row for the same thing. If the user says to stop, stop.

## How to write it

- The board is **6 rows of 22 characters** unless `list_boards` says otherwise (compact is 3 × 15, large is 8 × 30). Aim for 1–3 short lines. Text past the last row is cut off, and the tool's result tells you when that happened.
- Text is upper-cased, word-wrapped and centred. Use `\n` to break lines yourself.
- Only A–Z, 0–9, space and `! @ # $ ( ) - + & = ; : ' " % , . / ? °` show up. Emoji, accents and other symbols show as blanks.
- Add a **colour chip** with `color`: `green` for success, `red` for failure, `yellow` or `orange` for needs-attention, `blue` or `violet` for information.
- Lead with the outcome and leave out details the user can read in the terminal. `TESTS PASSED\n412 OK IN 3M` is better than a sentence.

## Which tool

- `show_status`: a **temporary** state, such as `DEPLOYING…` or `RUNNING TESTS`. It shows once and goes away after `seconds` (15 by default); the board then goes back to what it would otherwise show (the previous message, or the user's Sources rotation). Prefer it for anything that won't matter in a minute.
- `send_message`: a result that should **stay up** until something replaces it, like done, failed or needs you. It takes `board` to send to one of the user's other boards or groups by name. Call `list_boards` first if you don't know the names.
- `play_sequence`: a few frames in a row, such as a countdown. Use it rarely.
- `send_layout`: an exact grid of character codes (0–71), for pictures or precise placement.
- `get_board`: what the board shows now. Check it before replacing something the user put up themselves.

`send_message` and `show_status` take `priority: "alert"`, which still pops up while the user's Focus or Do Not Disturb holds other messages. Use it only when something needs the user now (a production outage, a failure they're waiting on), never for routine results.

## Examples

- After a 10-minute test run passes: `send_message` with text `TESTS PASSED` and color `green`.
- Blocked on a question: `send_message` with text `NEED YOUR INPUT\nAPI DESIGN` and color `yellow`.
- Starting a deploy you'll report on later: `show_status` with text `DEPLOYING` and seconds `120`.

If a tool reports that Rattleboard isn't reachable, tell the user once that they can open Rattleboard and turn on Settings → API server. Don't keep retrying.
