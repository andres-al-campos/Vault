# Vault

A small desktop app around [yt-dlp](https://github.com/yt-dlp/yt-dlp) for archiving a
YouTube channel: fetch a channel, see every video as a card, download the ones you want.
Downloads run one at a time and are recorded in an archive file, so running it again
later fetches only what's new.

Built with [Tauri v2](https://tauri.app) — a Rust backend that drives yt-dlp as a
subprocess, and the same HTML/CSS UI from the `sketches/004-cards-queue-sidebar` mockup.

## Requirements

- **yt-dlp** on the machine. The app looks in `~/.local/bin`, `/opt/homebrew/bin`, and
  `/usr/local/bin`, falling back to `yt-dlp` on `PATH`:
  ```sh
  brew install yt-dlp        # or: uv tool install yt-dlp
  ```
- **ffmpeg** — required by yt-dlp to merge separate video/audio streams.
  ```sh
  brew install ffmpeg
  ```
- **Rust** and **Node 18+** to build.

## Build and run

`build.sh` is the entry point, following the same convention as the other projects:

```sh
./build.sh          # package Vault.app, install to /Applications/_vibe_coded, launch
./build.sh dev      # hot-reload dev window
./build.sh test     # fast Rust unit tests
./build.sh test net # also the live tests that fetch real channels (~80s)
./build.sh clean    # drop dist/ and src-tauri/target/
```

The default replaces whatever copy is already in `/Applications/_vibe_coded`, ad-hoc
signs it, and launches it — so a rebuild is the copy that opens, never last build's.

## How it works

| Piece | Where | What it does |
|---|---|---|
| `fetch_channel` | `src-tauri/src/lib.rs` | Runs `yt-dlp --flat-playlist -J <url>` and parses the JSON into a video list (id, title, duration, views). One request gets the whole channel. |
| `download_video` | `src-tauri/src/lib.rs` | Spawns yt-dlp with `--download-archive` and a `--progress-template`, then streams parsed progress back as a Tauri event. |
| `cancel_download` | `src-tauri/src/lib.rs` | Sends `SIGTERM` to the running yt-dlp process. The partial `.part` file stays, so re-queuing resumes. |
| `load_state` / `save_state` | `src-tauri/src/lib.rs` | Reads and writes `~/.vault/state.json` — the remembered channels, which one is active, and each channel's download folder. |
| queue engine | `src/main.js` | Downloads strictly one at a time. Each finished job starts the next. |

### Remembered channels

The app writes `~/.vault/state.json` whenever anything changes (a fetch, a rename, a
folder choice). On launch it reloads that file and shows the channel you last had open,
straight from the cached video list — no re-fetch needed. The dropdown at the top-left
switches between remembered channels, and each row has an ✕ to forget one. Press
**Fetch channel** to refresh a channel's list with anything new.

### Download folders

Each channel has its own download folder. The first time you download a video from a
channel that doesn't have one yet, Vault asks where to put it — the picker opens on a
suggested `~/Downloads/Vault/<channel>` and creates it. The choice is remembered per
channel and can be changed any time from the folder button in the top bar or the chip
under the channel name. All of a channel's files land directly in its folder.

The **archive** at `~/.vault/archive.txt` is global, not per-channel: a video id is
recorded once, so re-fetching a channel skips anything already downloaded regardless of
which folder it landed in.

### Members-only videos

yt-dlp's flat channel listing marks restricted videos with
`availability: "subscriber_only"` — verified against a video literally titled
"(MEMBERS ONLY)" and against LTT, where 609 of 7171 listings carry it. So Vault
flags those cards with a **Members** badge and disables their Download button
rather than letting them fail every time. `Download all` skips them.

Note the tempting shortcut is wrong: **"no view count" does not mean
members-only.** Plenty of ordinary public videos come back with a null
`view_count` in flat mode (several Veritasium videos with 8M+ views, for
instance). `availability` is the signal that actually holds.

### Tabs, sorting, filters

The `Videos` tab is the default and is the only one that's fetched up front —
it already excludes shorts and past livestreams, so the main list stays clean.
`Shorts` and `Live` are fetched lazily the first time you click them, and kept
in memory only (not persisted).

Sorting: **Newest** (YouTube's own order), **Oldest** (reversed), **Most
popular** (`view_count`), **Shortest** / **Longest** (`duration`). There is no
upload date in flat mode — `timestamp` came back null on every video tested — so
newest/oldest fall back to the order YouTube returns, which is newest-first.
Shorts carry no duration, so they sort last under Shortest/Longest.

Filters: **All**, **Public** (not `subscriber_only`), **Members**
(`subscriber_only`), each with a live count.

### Failures and retries

A failed download shows a short classified reason instead of a generic error —
`Members only`, `Network error`, `Private`, `Age-restricted`, `Geo-blocked`,
`Unavailable`, `No downloadable format`, `Post-processing failed`. The
classification reads yt-dlp's own output; stderr is drained on its own thread,
since a piped-but-unread stderr is what would otherwise block yt-dlp once the
pipe buffer filled.

Failed rows get a **Retry** button, and the sidebar has **Retry failed** to
re-queue all of them. Cancelling works whether the job is running (SIGTERM to
yt-dlp) or still queued (dropped from the frontend's own queue, since a queued
job has no process to signal — this was the bug where ✕ appeared to do nothing).

### Why there's no pause button

yt-dlp has no pause. It only has `--continue`, which resumes a partial file after an
interruption. So "cancel" kills the process and re-queuing downloads resumes from where
it left off. A real pause would be `SIGSTOP`/`SIGCONT`, which is fiddlier and was left
out.

## Notes / not-done-yet

- `csp` is `null` in `tauri.conf.json` (no Content-Security-Policy). Fine for a local
  app; tighten before distributing.
- `Download all` queues every video in the channel without a cap. For a large channel
  that's a long queue — a batch-size limit would be a sensible addition.
- The archive is global, so changing a channel's folder after downloading doesn't
  re-fetch files that are now in the old location.

## Layout

```
youtube-vault/
├── build.sh              the entry point (package / dev / test / clean)
├── index.html            UI shell
├── src/
│   ├── main.js           fetch, queue, persistence, event wiring
│   └── styles.css        light & dark themes (CSS variables)
├── src-tauri/
│   ├── src/lib.rs        yt-dlp integration + saved state
│   ├── tauri.conf.json
│   └── icons/
└── sketches/             the HTML mockups this design came from
```