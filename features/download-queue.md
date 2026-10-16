# Download queue

Queue videos and watch them download one at a time in the sidebar.

## Sub-features

- One download runs at a time; the rest wait in the order they were added.
- A video that is queued, running or finished can't be queued twice. A failed
  one can.
- Cancel (✕) stops a running download or drops a waiting one. Re-queuing a
  cancelled download resumes its partial file.
- Pause all ends the running download and holds the line; the button becomes
  Resume all. The paused download keeps its partial file, goes back in
  line where its row is, reading "Paused at N%", and carries on from there
  on Resume. Resume also queues the failed rows again. A retried row keeps
  its place and takes its turn from there. Videos queued while paused wait for it, and every waiting row reads
  "Queued · Paused".
  With nothing left but failures the same button reads Retry N failed.
- A waiting row is dragged by its grip to a new place in the line.
- What hasn't finished is remembered when Vault closes: it comes back in the
  same order, and starts again at launch unless the queue was paused.
  Finished and failed rows aren't remembered.
- A failed row shows why and gets its own Retry and ✕.
- A finished row plays the video, as Play on its card does, from its
  thumbnail or its title, with ⌘-click opening it in the background; see
  [player.md](player.md). Its thumbnail carries the player's play mark at
  half strength, full on hover with the image dimmed. It works for a row from
  a channel that isn't on screen.
- A download stopped by YouTube's bot check doesn't fail. The check is on
  the network, not the video, so the whole line rests: the video goes back
  to its place, its row reads "Bot check · again at 14:32", the header says
  the same, and nothing starts until then. The waits are 1, 5, 15, 30 and 60
  minutes; a download that gets through resets them. After the last wait
  the row fails, saying the bot check is still there, and the line pauses,
  so the rest don't each meet it too. Pause all then Resume all ends a rest
  early; Resume all after the line gave up tries again.
- A failed download is unfinished: it counts as queue, not as finished. Clear
  finished removes done rows only. Clear queue asks, then drops everything
  waiting or failed and leaves the running download alone.
- The header is always the same three buttons, Pause all, Clear queue and Clear
  finished, greyed out while they have nothing to act on.
- Download again, on a card whose file is missing, is an ordinary download
  into the folder that recorded it.
- A download that ends with the file already in the folder reads "Already in
  archive": yt-dlp leaves a finished file alone.
- A download only counts once its file is in the folder. If yt-dlp reports
  success and no file is there, the row fails and says so.
- Download all N queues what the filters have narrowed to; see
  [browsing.md](browsing.md).
- Every row in the queue is the same height, so cancelling one puts the next
  row's ✕ under the pointer.
- The panel button at the end of the top bar hides the Downloads panel and
  shows it again; the channel takes the freed width. Hidden, the button
  carries the number of downloads still to finish (running, waiting or
  failed). Vault opens with the panel hidden, unless something was still in
  line when it last closed (waiting, paused or failed); then it opens with
  the panel showing. Queueing something when nothing else is left to do
  brings a hidden panel back; hidden while downloads run, it stays hidden.
  Downloads carry on either way.

- A finished download writes the video's record to the folder's
  `.vault/videos.json`: real upload date, length, views, members flag and when
  it was downloaded.

## How to get to it

A card's button on any tab; Download all N on the tab strip. The queue is the
right-hand sidebar.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): the queue's rules in `src/queue.test.js` —
order, one at a time, no double-queuing, retry and re-queue of a failed job, a
second completion report changing nothing, cancelling a waiting job, pause and
resume (including a resume before the paused download has exited), moving a
waiting job, clearing the queue, a saved queue coming back, and a bot check
resting the line, counting until a download gets through, and ending on
Resume. How a
finished download becomes a record, in the Rust unit tests.

`./build.sh check net` (~100s): really downloads two short videos from a live
channel into a temp folder and walks them through every archive state — not
archived, on disk with a full record, recorded but file lost, downloaded again
with one record kept, and skipped when the file is already there.

By click against the stand-in backend (see the README's driving conventions):
`?scenario=library&fail=<id>&speed=300` — Download on a Shorts card, Download
again on a missing file, its failure, Retry. No script replays this yet.

`?scenario=library&bot=7&speed=300` — the next 7 downloads meet the bot
check. Queue two, then Pause all and Resume all to end each rest.

By hand only: cancelling or pausing a running download for real (the stand-in
has no yt-dlp to end), the queue coming back after the app itself is quit, and
anything through the native folder picker.

Last driven: 2026-10-13, in the stand-in: pause, resume, reorder, clear and
restore. The live download test last passed on 2026-10-12 against @billwurtz
(12s on its own), before any of those existed.
The bot check, 2026-10-14 in the stand-in with `&bot=7`: the row and the
header read "Bot check · again at 7:49 PM", paused they read "Queued ·
Paused"; the waits came to 1, 5, 15, 30 and 60 minutes; the sixth check
failed the row ("still there after 6 tries") and paused the line with the
second video still queued; with the check gone, Resume all downloaded both.
The panel at launch, 2026-10-14 in the stand-in: an empty queue opened with
it hidden; a paused job left in storage opened it, reading "Paused · 1
waiting". Queueing onto an empty queue brought a hidden panel back; queueing
a second while the first ran left it hidden.
Play from a finished row, 2026-10-15 in the stand-in: two downloads finished,
both titles took the pointer, and clicking one opened it in a player tab
(`player_load` with its file). The play mark showed on finished rows only, at
0.5, centred on the thumbnail; clicking the thumbnail opened the player too. Not driven: a row from another channel, and a
click in the moment before the archive read lands.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

- 2026-10-13. A failed download is unfinished, not finished: Clear finished
  leaves it, Clear queue takes it, and Resume all tries it again.
- 2026-10-13. The Downloads header is the same three buttons at all times,
  greyed out when they have nothing to act on: four or five that came and
  went didn't look good.
- 2026-10-13. It's Pause all, not Stop all, since a download picks up where
  it left off. Waiting rows read "Queued · Paused" so nobody expects them to
  start.
- 2026-10-13. A retried or paused download keeps its row. Rows only move
  when dragged.
- 2026-10-14. A bot check waits and tries again by itself rather than
  asking for cookies: cookies tie the downloads to your account, which can
  get it flagged, so they're for when nothing else works (members-only
  videos). The waits and their number are an agent's guesses.
- 2026-10-14. The panel starts hidden when the queue is empty and showing
  when it isn't, instead of as it was left: you come to Vault to watch, and
  an empty panel took a third of the window to say nothing was queued.
  Queueing onto an empty queue opens it again, so a new download is seen.

## Gotchas

- The panel button: its place and icon, the count on it and the 0.2-second
  slide are an agent's guesses, as is "nothing left to do" meaning nothing
  waiting, running or failed (finished rows don't count) when a new download
  decides whether to bring the panel back. Clicked through in the stand-in on 2026-10-13, not
  in the app.
- The queue is saved in the web view's own storage, not in `~/.vault`: it is
  what this window was doing, not a fact about the library.
- Pause all, Clear queue and the grip were clicked through in the stand-in on
  2026-10-13. Whether a restored queue should start by itself at launch is an
  agent's guess.

- A queue row's buttons depend on its status, so a status change redraws the
  list. Patching the row in place once left a failed row with no Retry.

- A file ending `.part` or `.ytdl` doesn't count as on disk: it is a download
  that never finished.

- A download yt-dlp skips reports twice, the skip line and then the process
  exit. The first report wins.
- Download buttons resolve through the list the grid rendered. Looking a video
  up in one tab's list is what left Shorts and Live buttons dead.
- 2026-10-15. A finished row can be played, and its thumbnail shows it: the
  player's play mark at half strength. A clickable title alone gave no sign
  that anything could be done. The mark is the player's, not the card's ▶.
