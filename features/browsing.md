# Browsing a channel

A channel's videos as a grid of cards under a YouTube-style header, narrowed by
what you have and what you can get.

## Sub-features

- Header: banner, round avatar, name with a verified tick, handle, subscriber
  count, video count, and the download folder. The folder path is a link that
  opens the folder; Change picks another.
- Tabs: Videos, Shorts, Live.
- Filters, one row: All, Members, a line, Archived, Not archived. Members
  switches on and off by itself; Archived and Not archived take turns, since
  both on would be everything; All switches every one off and is lit while
  none is on. So Members + Not archived is the members-only videos left to
  download. Each option's number is what switching it on would show, the
  others as they are. Members is there even at 0. When they change, the
  numbers count to their new values over the fill's slide; on a new channel
  or tab they just appear.
- Sort: Newest, Oldest, Most popular, Shortest, Longest. It sits after the
  filters, on the left of their row, and is as wide as its longest choice
  whichever is picked.
- Search, at the right of the filter row, looks through the titles of the
  channel on screen. A title matches when it has every word typed, in any
  order, ignoring case and accents. It searches what the filters leave; the
  numbers on the filters stay those of the whole tab. The box says how many
  matches there are, and when the filters hide some, how many are on screen
  ("2 of 15"); then a "Show all 15" button appears to the left of the box. It
  switches the filters off, and then has nothing left to do and goes. ⌘F
  goes to the box; Esc clears it, and a second Esc leaves it. The search is
  kept across tabs, cleared on moving to another channel, and not
  remembered across launches.
- Download all N queues the matches too: it takes what's on screen.
- Each card's one button follows its state: Download, Queued, a percentage
  that fills the button, Play, Download again (file missing), Retry (last try
  failed), or Members only (disabled without cookies). An archived card's
  thumbnail also plays it.
- A missing file counts as archived and says so on its card.
- Archived videos the channel no longer lists stay on the Videos tab after the
  listed ones, marked "Missing on YT". One YouTube called unlisted when it
  was downloaded (one got from its link) has an "Unlisted" badge instead,
  where the members-only badge goes.
- Download all N queues everything the filters leave that isn't on disk or already
  queued, forcing the missing ones. Without cookies it leaves members-only
  videos out, and pointing at it says how many. With only locked members-only videos on
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
what search matches and that it leaves the counts alone, what
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
Search, 2026-10-14 in the stand-in: with Archived chosen, "the" showed 2
cards, "2 of 15" in the box and "Show all 15", with the filter numbers
unchanged; the box kept its cursor through each key; the button switched
the filters off and went, leaving "15"; Esc cleared the box. The sort button
measured 124px for all five choices. In the app, by the user: ⌘F and Esc
work, and search is fast on a big channel.
One row of filters, 2026-10-14 in the stand-in: from All, Members lit it
alone (Members 3, Archived 1, Not archived 2); Not archived lit both (2
cards); Archived took Not archived's place (Members 1); Members again left
Archived 7; All cleared both; Not archived twice went back to All. With
`&many=2000`, Members beside Not archived counted 1994 down to 101, the
fill's edge within 0.4px of the option's on every frame.

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
- 2026-10-14. Search is for the channel on screen only, for simplicity. It
  searches what the filters leave, with one button to search under All
  instead, which goes once pressed. The filters stay in view, and the sort
  moved to the left of the row to make room for the box on the right.
- 2026-10-14. A search doesn't change the numbers on the filters. Instead
  the box says how many of its matches are on screen, "2 of 15", and the
  button reads "Show all 15".
- 2026-10-14. The sort button keeps one width, its longest choice, with the
  text on the left: a width that changed with the choice moved the row.
- 2026-10-14. The chosen filter's outline is the group's full height, on
  the group's own border, so it is as tall as the sort button and search box
  beside it: a shorter box next to taller ones looked off.
- 2026-10-14. The filters are one row with one All. Two groups each with
  its own All was weird. Members and an archive state can still be on
  together. The line between Members and the archive pair was the user's
  condition for it: without it, which ones take turns would be confusing.
  It is two-thirds of the option's height: shorter, it looked too small.
  Public only, hiding members-only videos, went with it; Download all already
  skips the ones it can't get. Members shows even at 0.
- 2026-10-14. No Public option: with Download all skipping members-only
  videos it can't get, and Members there for anyone who has a membership,
  hiding members-only cards while browsing isn't worth a fifth option.
- 2026-10-14. The filter numbers count to a new value instead of changing
  at once. The count's curve and its 0.18s, the fill's, are an agent's
  guesses.
- 2026-10-14. The filter row's text is 13.5px, the Download button's size:
  at 12.5px the controls looked empty for their height.
- 2026-10-14. "Unlisted" is a badge where the members-only one goes, not a
  word under the title, so it takes no extra space. Its grey is an agent's
  guess.

## Gotchas

- "Unlisted" is what YouTube said when the video was downloaded. A video
  made private or deleted since still reads "Unlisted".
- Search: titles only, every word, ignoring accents; that the button shows
  only when the filters hide matches; Esc; clearing on a channel change and
  not being remembered: all an agent's guesses. At the default window with
  the Downloads panel open, the row can't hold the button
  and the box, so the box drops to a second line when the button appears.
  The user said that is fine on a small window.

- Only the rows near the view are on the page: the ones in view and a
  screen's worth above and below, swapped as it scrolls. The rest are empty
  space of the same height, so the scrollbar still shows the whole list. It
  relies on every card being the same height. Two things once broke that: a
  one-line title, and the 4:3 grey picture YouTube serves for a removed
  video. A card that isn't on the page can't be found by the browser's own
  find, or by code that looks a card up by id; such code has to cope with
  finding nothing.

- The filter row has two fills: one for All or Members, one for the archive
  pair. Each slides within its side; one with nowhere to go fades out and one
  that comes from nothing fades in. Their 0.18s and the line's grey are an
  agent's guesses.
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
- Numbers on the filters, and the download button riding the tab strip,
  are an agent's guesses at the layout.
