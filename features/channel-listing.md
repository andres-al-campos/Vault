# Channel listing

Type a channel and see its videos as cards.

## Sub-features

- Accepts an @handle, a bare name, a `UC…` channel id, or a pasted URL.
- However it was typed, a channel has one identity:
  `https://www.youtube.com/@handle` (or `/channel/…`, `/c/…`, `/user/…`). Host,
  tab, query, trailing slash and the handle's case don't make a second entry.
- Videos is fetched up front and saved. Shorts and Live are fetched the first
  time they're opened and kept in memory only.
- The listing also brings the channel's avatar, banner, subscriber count and
  verified flag, when YouTube has them.
- Sync re-fetches the Videos list of the channel on screen and rescans its
  folder. A finished file there with no record in `.vault/videos.json` gets
  one, so a video copied in by hand shows as archived. Records also take the
  length and views the listing has, so they outlive the listing. Titles are
  not recorded: a delisted video is titled by its file's name, and one whose
  file is also gone shows its id. A record whose file is gone stays, and
  its card says "File missing". The same rescan runs at launch, after each
  download and when the folder is changed.
- After a Sync, archived videos with no upload date recorded (downloaded
  before dates were kept, or copied in by hand) get theirs looked up in the
  background, one request each. A video YouTube no longer has gets none and
  is asked for again on the next Sync.
- The rescan takes only names ending in a YouTube id tag and one media
  extension, `Title [id].mp4`: not a part file, an unmerged `[id].f137.mp4`,
  a thumbnail, or "Song [Official Video].mp4".
- A channel with no Shorts or Live tab shows that tab as empty.
- A failed Shorts or Live fetch shows the error with Try again. A failed Sync
  shows an alert and leaves the saved list as it was.

## How to get to it

The field in the top bar adds a channel, with Enter or the + inside it, and is
empty otherwise. Sync, next to it, refreshes the channel on screen. The tabs
are under the channel header.

## Driving it

Preconditions: none for the default run; network and yt-dlp for `net`.

`./build.sh check` (~1s once built): URL forms and channel identity, entry
order, failure classification, what the folder rescan records and leaves
alone, the import of a folder that still has `archive.txt`, and a damaged
`videos.json` never being replaced, in the Rust unit tests.

`./build.sh check net` (~80s): fetches two real channels and checks the
members-only flag arrives in the listing.

By hand only: the tabs, and how a failed fetch looks.

Last driven: 2026-10-12, tests only. The `net` tests were not run that day.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

- 2026-10-12. Titles aren't recorded: YouTube videos get renamed.

## Gotchas

- `.vault/videos.json` is the only record of what a folder has archived.
  `archive.txt` and `dates.txt` in an older folder are read once to build it
  and never written again; deleting `videos.json` redoes that import.
- A `videos.json` that can't be parsed makes the folder read as holding
  nothing, and the reason only reaches the log. Nothing is written over it.
- The tab is remembered across launches but Shorts and Live aren't saved, so a
  remembered tab is fetched on arrival.
- The alert on a failed Sync is an agent's guess at where that message belongs.
