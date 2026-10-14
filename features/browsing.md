# Browsing a channel

A channel's videos as a grid of cards under a YouTube-style header, narrowed by
what you have and what you can get.

## Sub-features

- Header: banner, round avatar, name with a verified tick, handle, subscriber
  count, video count, and the download folder. The folder path is a link that
  opens the folder; Change picks another.
- Tabs: Videos, Shorts, Live.
- Two filters, one choice each, that narrow each other: All / Public / Members
  (only when the tab has members-only videos) and All / Archived / Not
  archived. Each option's number is what choosing it would show.
- Sort: Newest, Oldest, Most popular, Shortest, Longest.
- Each card's one button follows its state: Download, Queued, a percentage
  that fills the button, Play, Download again (file missing), Retry (last try
  failed), or Members only (disabled without cookies). An archived card's
  thumbnail also plays it.
- A missing file counts as archived and says so on its card.
- Archived videos the channel no longer lists stay on the Videos tab after the
  listed ones, marked "Missing on YT".
- Download all N queues everything the filters leave that isn't on disk or already
  queued, forcing the missing ones. With only locked members-only videos on
  screen it reads "Set cookies to download" and opens the cookie menu; with
  nothing to do there is no button.
- The arrow beside it picks how many the button queues: the first 10, 25, 50
  or 100 of those, in the order on screen, so the sort picks the batch: Oldest
  + First 50 is the 50 oldest not yet archived. Picking one only changes the
  button, which then reads "Download first 50" instead of "Download all N"; pressing the button queues
  them. The pick stays until another is made or Vault is closed, so pressing
  again queues the next 50. Sizes that wouldn't be smaller than everything
  aren't offered, and with fewer left than the pick the button queues them
  all.
- Queueing more than 100 at once asks first.
- Tab, filters and sort are remembered across launches.
- The grid is at most five cards across. On a window wide enough for more,
  the cards grow.

## How to get to it

Add or pick a channel. The header, tab strip, filter row and grid are the
main column.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): the filter combinations, the counts, what
Download all takes, unlisted videos, the sorts, and which rows of a long
grid are drawn, in `src/library.test.js`;
channel art and counts from a listing, and an unlisted video's title from its
file name, in the Rust unit tests.

By click against the stand-in backend (see the README's driving conventions):
`?scenario=library&speed=1500`, and `&many=2000` for a long grid. No script
replays this yet.

By hand only: how it looks, in both themes and at other window sizes.

Last driven: 2026-10-12, against the stand-in at 1240x820, light theme.
Archived showed 7 cards (6 Play, 1 Download again, 1 unlisted); Members + Not archived showed 2 locked cards and "Set cookies to
download", which opened the cookie menu; choosing a browser unlocked them. A
download filled its card button to 40%, read Archived when done, then Play.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

- 2026-10-12. Members-only badges stay green, as on YouTube's own thumbnails.
- 2026-10-12. A card keeps its footer button, so download progress shows on
  the card itself.
- 2026-10-12. "File missing" shows on the card only, not as a filter: a file
  can only be missing if it was archived.
- 2026-10-13. Download reads "Download all N": "shown" wasn't saying much.
- 2026-10-13. Five cards across at most: six was too many, and a sixth
  appearing and going as the Downloads panel closed and opened made the grid
  jitter. With the panel hidden the cards get bigger.
- 2026-10-13. The chosen filter has an outline in the text colour, white in
  the dark theme, around its tint: the tint alone was too low in contrast to
  see which one was chosen.

## Gotchas

- Only the rows near the view are on the page: the ones in view and a
  screen's worth above and below, swapped as it scrolls. The rest are empty
  space of the same height, so the scrollbar still shows the whole list. It
  relies on every card being the same height. Two things once broke that: a
  one-line title, and the 4:3 grey picture YouTube serves for a removed
  video. A card that isn't on the page can't be found by the browser's own
  find, or by code that looks a card up by id; such code has to cope with
  finding nothing.

- A filter row's fill slides from the last choice to the new one. It once
  always slid from the row's left edge: the fill was measured and styled
  after going into the page, and reading a size laid it out half-placed. No
  test covers it; watch each frame's position in the stand-in to check it.
  The slide starts two frames after the redraw, so a long grid being laid
  out doesn't use up its time. That part can't be seen in the stand-in: its
  channel is 60 videos and its browser times a slide differently.

- The "Queue N videos?" popup has not been seen: the stand-in's channel has 60
  videos, under the 100 that triggers it. The threshold and the batch sizes
  are an agent's guess, as is a video that is both "Missing on YT" and "File
  missing" still offering Download again, which will fail if YouTube really
  removed it.
- The header's art and subscriber count are saved with the channel. A channel
  saved before they were fetched shows its initial until the next Sync.
- An archived Short shows on the Videos tab as "Missing on YT" until
  Shorts has been opened once: that is when the app learns it is a Short.
- The date under a card is the real upload date once the video is archived.
  Until then it is approximate: YouTube's "3 weeks ago" turned into a date
  and back.
- Newest and Oldest sort by the order of the listing, which is upload order
  and exact. A "Missing on YT" video isn't in the listing: with an upload date
  it sits among the listed ones by date, without one it goes after them.
- Shorts use the same 16:9 cards as videos.
- Numbers on both filter groups, and the download button riding the tab strip,
  are an agent's guesses at the layout.
