# Subscriptions

A subscribed channel downloads its new uploads without being asked.

## Sub-features

- Subscribe in the channel header opens a popup: Videos (ticked), Shorts and
  Live. Subscribing needs a download folder and asks for one if there is none.
  Once subscribed, a tick takes effect as it's made, and the same popup
  unsubscribes.
- Each time Vault opens, every subscribed channel is refreshed in the
  background, one after another, and its new videos are queued. Sync on a
  subscribed channel does the same for that channel.
- New means listed above the newest archived video. The listing is newest
  first, so no dates are involved. A video skipped on purpose stays skipped
  once something newer is archived.
- With nothing archived, only videos that weren't in the list before the
  refresh count. Subscribing never queues the back catalogue.
- Members-only videos are queued like any other when cookies are set, and wait
  for a later refresh when they aren't.
- Shorts and Live have no saved list, so each is fetched on every refresh when
  ticked, and with nothing archived on that tab nothing is queued from it.

## How to get to it

The Subscribe button beside the channel name.

## Driving it

Preconditions: none for the default run.

`./build.sh check` (~1s once built): which videos count as new, with and
without an archived video, in `src/library.test.js`; a subscription surviving
the frontend's save, in the Rust unit tests.

By hand, in the browser against the stand-in (`drive/fake-backend.js`):
`?scenario=library&subscribed=1` queues one video at launch, the one above the
newest archived.

Last driven: 2026-10-12, against the stand-in. Subscribing from the popup
saved Videos only and the button read "✓ Subscribed"; Sync then queued 1
video; a launch with `&subscribed=1` queued 1. Not driven against YouTube.

## Gotchas

- A failed background refresh at launch only reaches the console. The saved
  list stays, and Sync on the channel shows the reason.
- Nothing is downloaded while Vault is closed.
- Queueing on Sync as well as at launch, and nothing happening at the moment
  of subscribing, are an agent's guesses.
