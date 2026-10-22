# Vault

You come to Vault to watch what you've kept. Archiving a channel is the chore
that makes that possible, so once it's set going it should ask for little
attention. Every part of the app serves that, and it's what to compare a
screen against when stepping back from it.

Read `features/README.md` before adding anything. `./build.sh check` runs the
laws and tests; plain `./build.sh` installs and launches the app.

## Principles

Look-and-feel calls that hold across the app, with the reason given at the
time, each starting with the time and version it was made or reworded at. A call about one feature is in that feature's file, under Calls.

- 2026-10-22 18:56, 0.1.0. Three tiers of attention, and a control's look says its tier.
  High is the white fill: Open videos, the way back to what's being watched,
  and nothing else. Mid is a grey fill: Download all. Low is a faint tint:
  the chosen filter (with a thin outline, so it can be told from the rest),
  and Subscribe, which is rarely clicked. Until then the
  white fill was on all of them (a 2026-10-12 call, for being easy to see),
  and with everything loud nothing was the main thing. The tiers are fixed:
  they don't change with what the page is being used for.
  `laws/one-loud-button.sh` keeps the white fill to one button.
- 2026-10-22 18:56, 0.1.0. Hover lights the border, on cards and on bordered buttons.
  Cards don't lift or gain a shadow: the raise wasn't liked.
- 2026-10-22 18:56, 0.1.0. No orange: orange on black is too strongly another site's look.
  Blue was tried on Download and looked too light and flat. Failures and
  retries are amber.
- 2026-10-22 18:56, 0.1.0. The logo is YouTube's red, and the app icon a red near it.
  Buttons aren't: a red button means something else.
- 2026-10-22 18:56, 0.1.0. One record per question: a channel folder's `.vault/videos.json`
  is the only thing that says a video is archived. yt-dlp is a dependency, not
  the thing being used, so its own archive file isn't kept up.
- 2026-10-22 18:56, 0.1.0. Nothing teleports: a thing that changes place or appears moves
  or fades there, since a jump is jarring. Menus always fade, the filter fill
  and the tab underline slide. Slides ease out: fast to leave, slow to settle.
  First said on 2026-10-12, of menus and filter rows appearing at once.
- 2026-10-22 18:56, 0.1.0. The best button is no button: with nothing to download there
  is no Download button, where there used to be a disabled one.
- 2026-10-22 18:56, 0.1.0. Nothing on the page moves when a view changes: controls keep
  one height and the content one width, with or without a scrollbar. A
  control whose label changes keeps the width of its longest label. Some
  movement while resizing, or on a small window where a row has to wrap, is
  fine (2026-10-14).
- 2026-10-22 18:56, 0.1.0. Open videos has no play icon, so it doesn't read as the same
  button as Resume all. Resume all is the same grey as the buttons beside
  it: it doesn't need to stand out.
- 2026-10-22 18:56, 0.1.0. The name is Vault. Where there is room it is written
  "Vault - Channel Archiver": the name, a short dash, and what it does, so
  nobody is misled. "YouTube" stays out of both and goes in the description.
- 2026-10-22 18:56, 0.1.0. The app icon is a red tile with a white play triangle turned
  to point down: a nod to YouTube's button without its rounded rectangle,
  which is the part that is theirs. The plain triangle looked too much like
  YouTube, so a cut in its top edge makes it a V. It is equilateral (narrower
  ones were tried and this looked better), barely rounded at the corners,
  and large, with the same room above it as below its point. The red is a
  little softer than YouTube's, which was too red in the Dock. Switched, a
  red V on white looked like a heart or a hospital sign.
- 2026-10-22 18:56, 0.1.0. The icon's V is opaque white: it's the one thing that shouldn't
  blend with what's behind it. Smaller, it blurred into the red and looked
  pink. macOS 26 also drew the old-format icon as glass and turned the V
  pink, so the app carries a layered icon as well (`build.sh`,
  `src-tauri/Vault.icon`) with the glass turned off.
- 2026-10-22 18:56, 0.1.0. The wordmark's V is a normal letter in red: with the icon's
  mark in its place the word read as "ault".
- 2026-10-22 18:56, 0.1.0. The loading spinner is an ensō: one brush stroke round a
  circle, thick where the brush comes down and thinning as it goes, asked
  for by that look. The open gap, its size and speed are an agent's guesses.
  Its width holds near the head and eases off (a cosine), since thinning
  straight after the head looked like a ball with a trail (2026-10-14). The
  more even taper felt better.
- 2026-10-22 18:56, 0.1.0. Cookies are a last resort: they tie what Vault does to your
  YouTube account, which can get it flagged. Vault works without them and
  waits out a bot check before suggesting them. Members-only videos are
  the one thing that needs them.
