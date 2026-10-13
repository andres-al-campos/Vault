# Player

Archived videos play inside Vault. Each one opens in a tab that keeps its
place, so several can be open at once and picked up later, the way YouTube
videos sit in browser tabs through a day.

## Sub-features

- Play on an archived card opens the video in a tab and shows the player.
  ⌘-click opens the tab without leaving the channel.
- "Open videos N" in the top bar goes back to the player. It shows only while
  tabs are open.
- Switching to a tab, or coming back through "Open videos", brings the video
  up paused where it was left; only Play on a card starts a video playing.
- While a video loads, the stage shows the frame that tab was left on, or its
  thumbnail if it hasn't been played yet, and fades to the video's first
  frame. The frame is saved when the tab is left.
- Tabs: click to switch, ✕ to close (shown on the tab on screen and the one
  under the pointer). Resting on a tab shows its whole title and its thumbnail,
  with a line along the thumbnail's bottom for how far through it is; once one such card is up, moving
  to another tab shows its card at once. Closing the tab on screen moves to the one on its right, or its left
  from the last one, and that tab waits paused where it was left. Closing the
  last tab returns to the channel.
- Controls: play/pause, time, a seek bar, volume and fullscreen. Clicking the picture also toggles
  play. Space or K toggles, ←/→ skip 5 seconds, J/L skip 10, M or the speaker
  mutes, Esc goes back to the channel.
- Fullscreen: the button at the right of the controls, or F. The window goes
  fullscreen and the tabs and controls lie over
  the picture, fading out with the pointer after 2.5 seconds without movement.
  They stay while the video is paused or the pointer is on them. F or Esc
  leaves; a second Esc goes back to the channel. A window that was already
  fullscreen stays so on the way out.
- Pointing along the seek bar shows the time a click there would land on.
- Starting or stopping playback blinks a play or pause mark in the middle of
  the picture, and muting or unmuting blinks a speaker the same way.
- The keyboard's play/pause key, headphone buttons and Control Center toggle
  the loaded video, through mpv's own media-key support.
- A video that reaches its end stays on its last frame and the play button
  becomes "Play again".
- Each tab resumes where it was left, across tab switches and relaunches. One
  left within 5 seconds of its end starts over.
- Going back to the channel pauses the video.
- Any format mpv plays, AV1 included. On a Mac without hardware AV1 it decodes
  in software.

## How to get to it

Play on any archived card (the thumbnail or the button under it), or "Open
videos" in the top bar.

## Driving it

`./build.sh check` (~15s): `src/player.test.js` covers the tab rules: one tab
per video, background opens, which tab takes over on close, per-tab resume,
starting over near the end, and reading saved tabs back, damaged ones included.

By click against the stand-in (`?scenario=library`): its player is a clock
with no picture, every video 40 seconds long. It shows opening, switching,
resume after a reload, replay at the end, Space, Esc and closing.

By hand only: the picture itself. mpv draws it in a view under the web view,
and neither the tests nor the stand-in have one.

Last driven: 2026-10-12, stand-in. Two tabs opened, one in the background;
switching away and back reloaded the first at 1.2s; after a reload the active
tab loaded at 18.8s; seeking to the end showed "Play again", which restarted
it from 0.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

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
- 2026-10-13. Fullscreen is the button or F. No double-click: two ways in
  are enough.

## Gotchas

- Needs mpv from Homebrew (`brew install mpv`). The build fails with that
  instruction when it's missing. The installed app loads libmpv from
  Homebrew's folder at launch, so removing mpv afterwards stops Vault from
  opening at all, not only the player.
- The web view is see-through over the stage, and the window behind it is
  black. The video view is moved to match the stage after the page resizes,
  so a fast resize can show black at the stage's edge for a frame.
- Keys pressed while the window is still animating into or out of fullscreen
  are lost: macOS takes the keyboard from the web view for the length of the
  animation. It's handed back every 100ms for 1.5 seconds after a switch.
- The window leaving fullscreen by itself (green button, system shortcut) is
  noticed a second after the resize settles, not at once.
- While the player is shown the video layer redraws on every display refresh,
  paused or not.
- Open tabs and their places live in the web view's local storage
  (`vault-open`), not in `videos.json`. Agent's guess: they're window state,
  like the theme. A "watched" mark on cards would want the place in the record
  instead.
- A tab remembers the file's path from when it was opened. If the file moves,
  the tab says the file isn't there until the video is played from its card
  again.
- Agent's guesses, never confirmed: pausing on the way back to the channel;
  ⌘-click for a background tab; starting over within 5
  seconds of the end; new tabs go at the right end; the stage is black while
  a video loads; the
  fullscreen keys and gestures, the 2.5-second fade, and keeping the tabs in
  fullscreen.
- The channel is hidden under the player with `visibility`, and the stage is
  see-through, so anything in the channel that is slow to hide shows over the
  video. A `transition: all` did exactly that to the clicked thumbnail;
  transitions are switched off under the player for that reason.
- The media key reaches mpv whether or not the player is on screen, so it can
  start the paused video's sound while the channel is showing. Its place is
  picked up again when the player is next opened.
- The saved frames are JPEGs in the system's temporary folder
  (`vault-stills/`), one per tab, replaced on each save. A closed tab's file
  is left for the system to clear.
- There is one player. Switching tabs unloads one file and loads the other,
  so the cover hides a real load; two videos are never live at once.
- One mpv log line at the first frame ("INVALID_FRAMEBUFFER_OPERATION") showed
  in the spike with no visible effect.
