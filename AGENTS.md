# Vault

Read `features/README.md` before adding anything. `./build.sh check` runs the
laws and tests; plain `./build.sh` installs and launches the app.

## Principles

Look-and-feel calls, with the reason given at the time.

- 2026-10-12. The chosen option in a row of filters is a solid fill, not an
  outline: the fill is easier to see. Subscribe uses the same fill.
- 2026-10-12. Hover lights the border, on cards and on bordered buttons.
  Cards don't lift or gain a shadow: the raise wasn't liked.
- 2026-10-12. Members-only badges stay green, as on YouTube's own thumbnails.
- 2026-10-12. A card keeps its footer button, so download progress shows on
  the card itself.
- 2026-10-12. "File missing" shows on the card only, not as a filter: a file
  can only be missing if it was archived.
- 2026-10-12. One record per question: a channel folder's `.vault/videos.json`
  is the only thing that says a video is archived. yt-dlp is a dependency, not
  the thing being used, so its own archive file isn't kept up.
- 2026-10-12. Titles aren't recorded: YouTube videos get renamed.
- 2026-10-12. The player's tabs look and feel like a browser's tab strip
  (Chrome's was the reference, without the site icon): a raised fill for the
  one on screen, hairlines between the rest, close on hover, the full title
  on a card below. Asked for by that look; the solid fill stays for filters.
  One departure: the hairline is kept beside the tab on screen as well. That
  tab floats with space above and below it, so without the line there was no
  telling where the tab to its left ended.
