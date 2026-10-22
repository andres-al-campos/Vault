# Vault features

Updated 2026-10-22 18:56, at 0.1.0.

What a user can do with Vault, how to reach it, and how to check it. Read this
before adding anything: if it's already here, extend it instead of building it
again.

This map covers the features changed so far, not the whole app. Folder choice
and cookies have no file yet.

| Feature | File | Platforms | Driveable headless? |
|---|---|---|---|
| Download queue | [download-queue.md](download-queue.md) | macOS | Tests, and by click against the stand-in (2026-10-12) |
| Channel listing | [channel-listing.md](channel-listing.md) | macOS | Tests, and by click against the stand-in (2026-10-12) |
| Browsing a channel | [browsing.md](browsing.md) | macOS | Tests, and by click against the stand-in (2026-10-12) |
| Remembered channels | [remembered-channels.md](remembered-channels.md) | macOS | Tests, and by click against the stand-in (2026-10-12) |
| Subscriptions | [subscriptions.md](subscriptions.md) | macOS | Tests, and by click against the stand-in (2026-10-12) |
| Player | [player.md](player.md) | macOS | Tab rules by tests and the stand-in (2026-10-12); the picture by hand |

## What counts as a feature

Something a user sets out to do, with its own way in. What you only reach
inside a feature is a sub-feature of it.

## Each file has

A stamp under the title (`Updated 2026-10-22 19:00, at 0.1.0.`: when the
description last changed, and Vault's version then), What the user gets,
Sub-features, How to get to it, Driving it, Gotchas. A file also has Calls when
a look-and-feel call was made about that feature alone: the decision and the
reason given. The pre-commit hook in `hooks/` writes the stamp, and starts each
Principle and Call with the time and version it was made or reworded at; a
change only to the Calls leaves the stamp alone. A call that holds across the
app is in `AGENTS.md`, under Principles.

## Keeping it current

A change to a feature updates its file in the same commit. `laws/feature-map.sh`
fails `./build.sh check` when a row has no file, a file has no row, or a file
names a `./build.sh` command that doesn't exist.

Before acting on a file, run `git log --since="<stamp time>"` on the code
you're about to touch. Where it moved after the stamp, read the file against it
and fix what's no longer true; the fix moves the stamp. `./build.sh` turns the
hook on. `laws/doc-stamps.sh` fails when a feature file or the README has no
stamp, or a stamp, Principle or Call lacks a time and a version Vault has had.

## Driving conventions

`./build.sh check` runs every check: the frontend's pure modules under
`node --test` and the Rust unit tests. It needs no network and no running app.
`./build.sh check net` adds the live tests, which fetch real channels (~80s).

To click through the UI, start the `vault-drive` server from
`.claude/launch.json` (`npx vite --port 1431 --strictPort`) and open
`http://localhost:1431/?scenario=library` in a browser. Outside Tauri the page
loads a stand-in backend, `drive/fake-backend.js`, which answers every command
from a recorded channel listing and plays downloads on a timer. Its header
lists the scenarios and switches (`fresh`, `library`, `corrupt`, `failtab=`,
`fail=`, `speed=`, `fixture=nasa`), and a clip in `drive/media/` gives its
player a picture (see [player.md](player.md)), and `window.__drive.log` records every command the page
sent. This proves the frontend only: the real yt-dlp path is the Rust tests.

The packaged app's own window can't be driven; what the stand-in can't show
(the native folder picker, the player's picture) is checked by hand.

The app's own data is `~/.vault/state.json`. Tests write to temp directories
and never touch it.
