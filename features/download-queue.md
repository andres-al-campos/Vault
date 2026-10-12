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
- Download again, on an Archive card whose file is missing, runs forced: yt-dlp
  ignores the archive record for that run and the record is re-stamped once the
  file lands. The job stays forced through any retry.
- Download all queues every video on the Videos tab that isn't archived,
  skipping members-only ones unless a cookie browser is set.

## How to get to it

A card's Download button on the Videos, Shorts or Live tab; Download all in the
channel header; Download again on an Archive card. The queue is the right-hand
sidebar.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): the queue's rules in `src/queue.test.js` —
order, one at a time, no double-queuing, force surviving a retry and a re-queue
from a card, a second completion report changing nothing, cancelling a waiting
job.

By hand only: that each button reaches the queue, progress, and cancelling a
running download.

Last driven: 2026-10-12, tests only. Not run in the app since the queue moved
into its own module.

## Gotchas

- A download yt-dlp skips reports twice, the skip line and then the process
  exit. The first report wins.
- Download buttons resolve through the list the grid rendered. Looking a video
  up in one tab's list is what left Shorts and Live buttons dead.
- Download all always means the Videos tab, whichever tab is showing.
- Clicking a card that says "In archive" queues it anyway; yt-dlp skips it and
  the row reads "Already in archive".
