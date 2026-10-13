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
  No hairline beside the tab on screen: it looked better without. So that
  tab's edge can mark where its neighbour ends, the bar leaves little space
  above and below it.
- 2026-10-12. The player's top and bottom bars are the same height, and
  short: more of the window goes to the video.
- 2026-10-12. The video is shown as sRGB, not pushed to the screen's full
  gamut: the more saturated picture looked unrealistic.
- 2026-10-12. The seek bar is YouTube's red, as an extra touch of YouTube. The
  same red marks progress on a tab's hover card.
- 2026-10-12. No orange: orange on black is too strongly another site's look.
  Download is the same white fill as Subscribe and the chosen filter, for now:
  blue was tried there and looked too light and flat. Failures and retries
  are amber.
- 2026-10-12. The logo is YouTube's red, and the app icon a red near it.
  Buttons aren't: a red button means something else.
- 2026-10-12. Menus and the filter rows animate, briefly: appearing at once
  was jarring.
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
- 2026-10-13. The best button is no button: with nothing to download there
  is no Download button, where there used to be a disabled one.
- 2026-10-13. Nothing on the page moves when a view changes: controls keep
  one height and the content one width, with or without a scrollbar.
- 2026-10-13. Open videos has no play icon, so it doesn't read as the same
  button as Resume all. Resume all is the same grey as the buttons beside
  it: it doesn't need to stand out.
- 2026-10-13. Nothing teleports: a thing that changes place or appears moves
  or fades there, since a jump is jarring. Menus always fade, the filter fill
  and the tab underline slide. Slides ease out: fast to leave, slow to settle.
- 2026-10-13. Fullscreen is the button or F. No double-click: two ways in
  are enough.
- 2026-10-13. The app icon is a red tile with a white play triangle turned
  to point down: a nod to YouTube's button without its rounded rectangle,
  which is the part that is theirs. The plain triangle looked too much
  like YouTube, so a cut in its top edge makes it a V. It is
  equilateral: narrower ones were tried and this looked better. Its corners
  are barely rounded. Its red is a little softer than YouTube's, which was too red in
  the Dock.
- 2026-10-13. The icon's V is large and centred top to bottom, with the same
  room above it as below its point: smaller, the white blurred into the red
  and looked pink.
- 2026-10-13. The icon's V is opaque white. It's the one thing that shouldn't
  blend with what's behind it: macOS 26 drew the old-format icon as glass
  and the V went pink, so the app also carries a layered icon (`build.sh`,
  `src-tauri/Vault.icon`) with the glass turned off.
- 2026-10-13. The icon stays a red tile with a white V: switched, a red V on
  white looked like a heart or a hospital sign. The wordmark's V is a
  normal letter in red: with the icon's mark in its place the word read as
  "ault".
- 2026-10-13. The name is Vault. Where there is room it is written
  "Vault — Channel Archiver": the name, a dash, and what it does, so nobody
  is misled. "YouTube" stays out of both and goes in the description.
- 2026-10-13. Download reads "Download all N": "shown" wasn't saying much.
