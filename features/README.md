# Vault features

What a user can do with Vault, how to reach it, and how to check it. Read this
before adding anything: if it's already here, extend it instead of building it
again.

This map covers the features changed so far, not the whole app. Folder choice,
the Archive tab, cookies, sorting and filters have no file yet.

| Feature | File | Platforms | Driveable headless? |
|---|---|---|---|
| Download queue | [download-queue.md](download-queue.md) | macOS | Tests only (2026-10-12) |
| Channel listing | [channel-listing.md](channel-listing.md) | macOS | Tests only (2026-10-12) |
| Remembered channels | [remembered-channels.md](remembered-channels.md) | macOS | Tests only (2026-10-12) |

## What counts as a feature

Something a user sets out to do, with its own way in. What you only reach
inside a feature is a sub-feature of it.

## Each file has

What the user gets, Sub-features, How to get to it, Driving it, Gotchas.

## Keeping it current

A change to a feature updates its file in the same commit. `laws/feature-map.sh`
fails `./build.sh check` when a row has no file, a file has no row, or a file
names a `./build.sh` command that doesn't exist.

## Driving conventions

`./build.sh check` runs every check: the frontend's pure modules under
`node --test` and the Rust unit tests. It needs no network and no running app.
`./build.sh check net` adds the live tests, which fetch real channels (~80s).

Nothing drives the window itself. What only shows on screen — a button that
does nothing, an error that reads as an empty list — is checked by hand, and
each file says which parts those are.

The app's own data is `~/.vault/state.json`. Tests write to temp directories
and never touch it.
