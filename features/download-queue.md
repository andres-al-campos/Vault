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
- Download N shown queues what the filters have narrowed to; see
  [browsing.md](browsing.md).
- Every row in the queue is the same height, so cancelling one puts the next
  row's ✕ under the pointer.

- A finished download writes the video's record to the folder's
  `.vault/videos.json`: real upload date, length, views, members flag and when
  it was downloaded.

## How to get to it

A card's button on any tab; Download N shown on the tab strip. The queue is the
right-hand sidebar.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): the queue's rules in `src/queue.test.js` —
order, one at a time, no double-queuing, retry and re-queue of a failed job, a
second completion report changing nothing, cancelling a waiting job, pause and
resume (including a resume before the paused download has exited), moving a
waiting job, clearing the queue, and a saved queue coming back. How a
finished download becomes a record, in the Rust unit tests.

`./build.sh check net` (~100s): really downloads two short videos from a live
channel into a temp folder and walks them through every archive state — not
archived, on disk with a full record, recorded but file lost, downloaded again
with one record kept, and skipped when the file is already there.

By click against the stand-in backend (see the README's driving conventions):
`?scenario=library&fail=<id>&speed=300` — Download on a Shorts card, Download
again on a missing file, its failure, Retry. No script replays this yet.

By hand only: cancelling or pausing a running download for real (the stand-in
has no yt-dlp to end), the queue coming back after the app itself is quit, and
anything through the native folder picker.

Last driven: 2026-10-12. The live download test passed against @billwurtz
(12s on its own) after the move to `videos.json`. The stand-in has not been
clicked through since.

## Gotchas

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
