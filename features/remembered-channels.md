# Remembered channels

Vault reopens on the channel you last had open, with its list and its download
folder.

## Sub-features

- Fetched channels, the active one, each channel's folder and the cookie
  browser are saved to `~/.vault/state.json` after every change.
- The selector at the top left switches channels; ✕ on a row forgets one.
- A state file that can't be read or parsed is never replaced. Vault shows the
  error and how to recover, and disables fetching until it's fixed or moved
  aside.
- Saved channel URLs are brought to their canonical form on load. Entries that
  turn out to be one channel merge into the first, keeping a folder if either
  had one.

## How to get to it

Launch the app; the channel selector in the top bar.

## Driving it

Preconditions: none.

`./build.sh check` (~1s once built): a missing file is a first run, a damaged
file errors and is left byte-for-byte as it was, saved URLs load canonical and
duplicates merge, in the Rust unit tests.

By hand only: the selector, and the screen shown for a damaged file.

Last driven: 2026-10-12, tests only.

## Gotchas

- The frontend holds the whole state and writes all of it back on each change.
  Anything that loads as "empty" is one save away from replacing the real file,
  which is why reading errors instead.
- Forgetting a channel drops its saved folder. The folder itself still holds
  its archive and a `.vault/channel.json` naming the channel; fetching the
  channel again and choosing that folder re-attaches it.
