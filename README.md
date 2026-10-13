# Vault

A small desktop app around [yt-dlp](https://github.com/yt-dlp/yt-dlp) for archiving a
YouTube channel: fetch a channel, see every video as a card, download the ones you want.
Downloads run one at a time and are recorded in an archive file in the channel's folder,
so running it again later fetches only what's new.

Built with [Tauri v2](https://tauri.app): a Rust backend that drives yt-dlp as a
subprocess, and a plain HTML/CSS/JS frontend laid out like a YouTube channel page.
[features/](features/README.md) says what each feature does and how it is checked.

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
- **mpv** — the in-app player is built on its library, which plays what the
  system's own player can't (AV1 on a Mac without hardware AV1).
  ```sh
  brew install mpv
  ```
- **Rust** and **Node 18+** to build.

## Build and run

`build.sh` is the entry point, following the same convention as the other projects:

```sh
./build.sh           # package Vault.app, install to /Applications/_vibe_coded, launch
./build.sh dev       # hot-reload dev window
./build.sh check     # every check: JS and Rust unit tests, the feature-map law
./build.sh check net # also the live tests, which fetch real channels and download two short videos
./build.sh clean     # drop dist/ and src-tauri/target/
```

The default replaces whatever copy is already in `/Applications/_vibe_coded`, ad-hoc
signs it, and launches it — so a rebuild is the copy that opens, never last build's.

## How it works

| Piece | Where | What it does |
|---|---|---|
| `fetch_channel` | `src-tauri/src/lib.rs` | Runs `yt-dlp --flat-playlist -J <url>` and parses the JSON into a video list (id, title, duration, views) plus the channel's avatar, banner and subscriber count. One request gets the whole channel. |
| `download_video` | `src-tauri/src/lib.rs` | Spawns yt-dlp with a `--progress-template`, streams parsed progress back as a Tauri event, and records the finished video in the folder's `.vault/videos.json`. |
| `cancel_download` | `src-tauri/src/lib.rs` | Sends `SIGTERM` to the running yt-dlp process. The partial `.part` file stays, so re-queuing resumes. |
| `list_archive` | `src-tauri/src/lib.rs` | Reads each channel folder's archive file and matches every recorded id to its file on disk. |
| `load_state` / `save_state` | `src-tauri/src/lib.rs` | Reads and writes `~/.vault/state.json` — the remembered channels, which one is active, and each channel's download folder. |
| player | `src-tauri/src/player.rs` | Runs libmpv and draws its picture in a view under the web view, which is transparent where the page leaves a hole for it. |
| open tabs | `src/player.js` | Which videos are open, which is on screen, and where each was left. |
| queue | `src/queue.js` | Which jobs exist and which runs next. Downloads strictly one at a time. |
| filters | `src/library.js` | Which videos the grid shows, in what order, and each one's archive state. |
| screen | `src/main.js` | Draws the header, tabs, filters, cards and queue, and wires them to the backend. |

### Remembered channels

The app writes `~/.vault/state.json` whenever anything changes (a fetch, a rename, a
folder choice). On launch it reloads that file and shows the channel you last had open,
straight from the cached video list — no re-fetch needed. The dropdown at the top-left
switches between remembered channels, and each row has an ✕ to forget one. The
**Sync** button refreshes the channel on screen: new uploads from YouTube, and
the folder rescanned. A video file copied into the folder with its `[id]` tag
shows as archived; one deleted from it shows "File missing".

### Download folders

Each channel has its own download folder. The first time you download a video from a
channel that doesn't have one yet, Vault asks where to put it — the picker opens on a
suggested `~/Downloads/Vault/<channel>` and creates it. The choice is remembered per
channel and can be changed any time with the folder button beside its path, under the
channel name. All of a channel's files land directly in its folder.

A channel's **archive** lives in its folder, at `.vault/videos.json`: one object
per archived video, with its real upload date, length, views and when it was
downloaded. Titles aren't recorded, since a channel can rename a video: a card
shows the listing's title, and a delisted video the one in its file's name. It is the only record of what's archived, and it sits next to a
`.vault/channel.json` naming the channel. A folder from before it, with yt-dlp's
`archive.txt`, is imported the first time it's read and that file is left as it
was. The folder is self-describing: move it to
another drive, point the channel at it again, and Vault knows what it holds. A video
counts as archived only for the channel whose folder records it.

### Members-only videos

yt-dlp's flat channel listing marks restricted videos with
`availability: "subscriber_only"` — verified against a video literally titled
"(MEMBERS ONLY)" and against LTT, where 609 of 7171 listings carry it. Vault
flags those cards with a **Members only** badge. Without cookies their Download
button is disabled rather than letting them fail every time; pick the browser
you're signed into YouTube with from the cookie button in the top bar and they
download like any other video.

Note the tempting shortcut is wrong: **"no view count" does not mean
members-only.** Plenty of ordinary public videos come back with a null
`view_count` in flat mode (several Veritasium videos with 8M+ views, for
instance). `availability` is the signal that actually holds.

### Tabs, filters, sorting

The `Videos` tab is the default and is the only one that's fetched up front —
it already excludes shorts and past livestreams, so the main list stays clean.
`Shorts` and `Live` are fetched the first time you open them, and kept in
memory only (not persisted).

Two filters, each with one choice at a time, narrow each other:

- **All / Public / Members** — shown only when the channel has members-only videos.
- **All / Archived / Not archived** — a video whose file has gone missing still
  counts as archived; its card says "File missing" and offers Download again.

The number on each option is what choosing it would show. **Download N shown**
queues everything on screen that isn't already on disk. The arrow beside it
sets the button to queue only the first 10, 25, 50 or 100 in the order shown,
and queueing more than 100 at once asks first.

Archived videos the channel no longer lists (deleted or made private) stay on
the Videos tab, after the listed ones, marked "Missing on YT".

Sorting: **Newest** (YouTube's own order), **Oldest** (reversed), **Most
popular** (`view_count`), **Shortest** / **Longest** (`duration`). Newest and
Oldest use the order YouTube returns, which is newest-first and exact. The
date under a card is approximate until the video is archived: the listing only
carries YouTube's "3 weeks ago". A download records the real upload date, and
Sync looks it up for archived videos that lack one.

**Subscribe**, in the channel header, queues a channel's new uploads each time
Vault opens or you press Sync: everything listed above the newest video you've
archived. Its popup picks which of Videos, Shorts and Live that covers.
Shorts carry no duration, so they sort last under Shortest/Longest.

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
- Changing a channel's folder reads that folder's archive: videos downloaded into
  the old folder read as not archived until the folder is switched back or the
  files are moved across.
- An archived Short shows on the Videos tab as "Missing on YT" until the
  Shorts tab has been opened once, since that is when Vault learns it is a Short.

## Layout

```
youtube-vault/
├── build.sh              the entry point (package / dev / check / clean)
├── index.html            UI shell
├── src/
│   ├── boot.js           entry: loads the app, and the stand-in backend on a dev server
│   ├── main.js           the screen: header, tabs, filters, cards, queue
│   ├── queue.js          download queue rules (+ queue.test.js)
│   ├── library.js        filter, sort and archive-state rules (+ library.test.js)
│   ├── player.js         open-video tabs and resume rules (+ player.test.js)
│   └── styles.css        light & dark themes (CSS variables)
├── src-tauri/
│   ├── src/lib.rs        yt-dlp integration, archive, saved state
│   ├── src/player.rs     the in-app player: libmpv under the web view
│   ├── tauri.conf.json
│   └── icons/
├── drive/                stand-in backend for clicking through the UI in a browser
├── features/             what each feature does and how it's checked
├── laws/                 checks run by ./build.sh check
└── sketches/             the HTML mockups the first design came from
```
