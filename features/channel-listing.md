# Channel listing

Type a channel and see its videos as cards.

## Sub-features

- Accepts an @handle, a bare name, a `UC…` channel id, or a pasted URL.
- However it was typed, a channel has one identity:
  `https://www.youtube.com/@handle` (or `/channel/…`, `/c/…`, `/user/…`). Host,
  tab, query, trailing slash and the handle's case don't make a second entry.
- Videos is fetched up front and saved. Shorts and Live are fetched the first
  time they're opened and kept in memory only.
- Sync re-fetches the Videos list of the channel on screen.
- A failed Shorts or Live fetch shows the error with Try again. A failed Sync
  shows an alert and leaves the saved list as it was.

## How to get to it

The search field and Fetch in the top bar; the tabs under the channel header;
the Sync button next to Fetch.

## Driving it

Preconditions: none for the default run; network and yt-dlp for `net`.

`./build.sh check` (~1s once built): URL forms and channel identity, entry
order, and failure classification, in the Rust unit tests.

`./build.sh check net` (~80s): fetches two real channels and checks the
members-only flag arrives in the listing.

By hand only: the tabs, and how a failed fetch looks.

Last driven: 2026-10-12, tests only. The `net` tests were not run that day.

## Gotchas

- The tab is remembered across launches but Shorts and Live aren't saved, so a
  remembered tab is fetched on arrival.
- The alert on a failed Sync is an agent's guess at where that message belongs.
