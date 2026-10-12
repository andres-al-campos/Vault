# Download queue

Queue videos and watch them download one at a time in the sidebar.

## Sub-features

- One download runs at a time; the rest wait in the order they were added.
- A video that is queued, running or finished can't be queued twice. A failed
  one can.
- Cancel (✕) stops a running download or drops a waiting one. Re-queuing a
  cancelled download resumes its partial file.
- A failed row shows why and gets Retry; Retry failed re-queues every failed row.
- Clear finished removes done and failed rows.
- Download again, on a card whose file is missing, runs forced: yt-dlp
  ignores the archive record for that run and the record is re-stamped once the
  file lands. The job stays forced through any retry.
- Download N shown queues what the filters have narrowed to; see
  [browsing.md](browsing.md).

## How to get to it

A card's button on any tab; Download N shown on the tab strip. The queue is the
right-hand sidebar.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): the queue's rules in `src/queue.test.js` —
order, one at a time, no double-queuing, force surviving a retry and a re-queue
from a card, a second completion report changing nothing, cancelling a waiting
job.

`./build.sh check net` (~100s): really downloads two short videos from a live
channel into a temp folder and walks them through every archive state — not
archived, on disk, recorded but file lost, an ordinary download skipping the
lost one, a forced one restoring it and recording the id once.

By click against the stand-in backend (see the README's driving conventions):
`?scenario=library&fail=<id>&speed=300` — Download on a Shorts card, Download
again on a missing file, its failure, Retry. No script replays this yet.

By hand only: cancelling a running download, and anything through the native
folder picker.

Last driven: 2026-10-12. The live download test passed against @billwurtz
(12s on its own). Against the stand-in: a Shorts download completed and its
card read "In archive"; a failed Download again showed Retry, the retry was
sent with force, and the Archive tab went from 1 not found to 0.

## Gotchas

- A queue row's buttons depend on its status, so a status change redraws the
  list. Patching the row in place once left a failed row with no Retry.

- A file ending `.part` or `.ytdl` doesn't count as on disk: it is a download
  that never finished.

- A download yt-dlp skips reports twice, the skip line and then the process
  exit. The first report wins.
- Download buttons resolve through the list the grid rendered. Looking a video
  up in one tab's list is what left Shorts and Live buttons dead.
