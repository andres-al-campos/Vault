# Channel listing

Type a channel and see its videos as cards.

## Sub-features

- Accepts an @handle, a bare name, a `UC…` channel id, or a pasted URL, with
  or without its `https://`.
- A link to one video (a watch page, youtu.be, a Short, a live page or an
  embed) opens that video's channel, adding it first if it isn't saved. A
  channel saved by its handle or by its id is the same channel. The video's
  title, as the channel's list gives it, goes in the channel's search box (a
  video looked up on its own can come back under another title, a
  translation say, which found nothing), the filters go off and the
  tab is the one that lists it, so its card is what's on screen. Nothing is
  downloaded until its button is pressed. While it works the page reads
  "Finding the video's channel…", then the usual "Fetching…" for a new
  channel, then "Finding the video…". A saved channel that doesn't list the
  video is synced once first. A video no tab lists says so, and says it's
  unlisted when YouTube does, with a button to download it into the
  channel's folder anyway. Once downloaded its card has an "Unlisted" badge; see
  [browsing.md](browsing.md).
- However it was typed, a channel has one identity:
  `https://www.youtube.com/@handle` (or `/channel/…`, `/c/…`, `/user/…`). Host,
  tab, query, trailing slash and the handle's case don't make a second entry.
- Videos is fetched up front and saved. Shorts and Live are fetched the first
  time they're opened and kept in memory only.
- The listing also brings the channel's avatar, banner, subscriber count and
  verified flag, when YouTube has them.
- Sync re-fetches the Videos list of the channel on screen and rescans its
  folder. While it runs the button is tinted and its arrow turns; a second
  click does nothing; when it's done the arrow finishes its turn and stops. A finished file there with no record in `.vault/videos.json` gets
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

The field in the top bar adds a channel or opens a video's, with Enter or the
+ inside it, and is empty otherwise. Sync, next to it, refreshes the channel on screen. The tabs
are under the channel header.

## Driving it

Preconditions: none for the default run; network and yt-dlp for `net`.

`./build.sh check` (~1s once built): URL forms and channel identity, which
links are one video and the channel a video names, entry order, failure classification, what the folder rescan records and leaves
alone, the import of a folder that still has `archive.txt`, and a damaged
`videos.json` never being replaced, in the Rust unit tests.

`./build.sh check net` (~80s): fetches two real channels and checks the
members-only flag arrives in the listing.

By hand only: the tabs, and how a failed fetch looks.

Last driven: 2026-10-12, tests only. The `net` tests were not run that day.
Video links, 2026-10-14 in the stand-in (`?scenario=library`, and `fresh` for
a channel not yet saved): a watch link with Not archived chosen opened Videos
with "in the future" searched, both filters at All, "1" in the box; an
unknown id showed "An unlisted video" and its button queued it, after which
its card had the "Unlisted" badge and Play; a failing
link said "Couldn't open that video" and kept the link in the field; from
`fresh` the link added the channel and searched the video. With
`&retitle=1`, where the lookup's title differs from the list's, the box got
the list's title and found the card. yt-dlp looked up
one real video in about 2 seconds with the fields the lookup reads.
Sync, 2026-10-14 in the stand-in: clicked twice, one fetch went out; the
button was tinted with its arrow turning, and stopped at the end of the
turn, about a second in.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

- 2026-10-12. Titles aren't recorded: YouTube videos get renamed.
- 2026-10-14. Sync shows it's working: the arrow turns and the button
  changes colour, both asked for, since a click with nothing to see read as
  ignored. A grey tint was too slight; red seemed off, and the button is
  already white. It takes the green of a card mid-download (an agent's
  pick), so working looks one way across the app. One turn a second and
  finishing the turn are an agent's guesses.

## Gotchas

- `.vault/videos.json` is the only record of what a folder has archived.
  `archive.txt` and `dates.txt` in an older folder are read once to build it
  and never written again; deleting `videos.json` redoes that import.
- A `videos.json` that can't be parsed makes the folder read as holding
  nothing, and the reason only reaches the log. Nothing is written over it.
- The tab is remembered across launches but Shorts and Live aren't saved, so a
  remembered tab is fetched on arrival.
- The alert on a failed Sync is an agent's guess at where that message belongs.
- Video links: searching by the video's title can show a few cards when
  titles share words. Setting both filters to All, syncing a saved channel
  once when the video isn't listed, and the unlisted message are an agent's
  guesses. A channel saved by an old `/c/` or `/user/` name won't be matched
  by a video link and gets a second entry by its handle. Opening a Short on
  the Shorts tab isn't seen in the stand-in, whose Shorts reuse ids from its
  Videos list.
