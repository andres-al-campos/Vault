# Player

Archived videos play inside Vault. Each one opens in a tab that keeps its
place, so several can be open at once and picked up later, the way YouTube
videos sit in browser tabs through a day.

## Sub-features

- Play on an archived card opens the video in a tab and shows the player.
  ⌘-click opens the tab without leaving the channel.
- "Open videos N" in the top bar goes back to the player. It shows only while
  tabs are open.
- Tabs: click to switch, ✕ to close (shown on the tab on screen and the one
  under the pointer). The line under a title is how far through it is. Resting
  on a tab shows its whole title and its time; once one such card is up, moving
  to another tab shows its card at once. Closing the tab on screen moves to the one on its right, or its left
  from the last one; closing the last tab returns to the channel.
- Controls: play/pause, time, a seek bar, volume, and a button that opens the
  file in the Mac's default player instead. Clicking the picture also toggles
  play. Space or K toggles, ←/→ skip 5 seconds, J/L skip 10, Esc goes back to
  the channel.
- Fullscreen: the button at the right of the controls, F, or a double-click on
  the picture. The window goes fullscreen and the tabs and controls lie over
  the picture, fading out with the pointer after 2.5 seconds without movement.
  They stay while the video is paused or the pointer is on them. F or Esc
  leaves; a second Esc goes back to the channel. A window that was already
  fullscreen stays so on the way out.
- Starting or stopping playback blinks a play or pause mark in the middle of
  the picture.
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

## Gotchas

- Needs mpv from Homebrew (`brew install mpv`). The build fails with that
  instruction when it's missing. The installed app loads libmpv from
  Homebrew's folder at launch, so removing mpv afterwards stops Vault from
  opening at all, not only the player.
- The web view is see-through over the stage, and the window behind it is
  black. The video view is moved to match the stage after the page resizes,
  so a fast resize can show black at the stage's edge for a frame.
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
  a video loads; the tab's hover card shows time where a browser shows the site; the
  fullscreen keys and gestures, the 2.5-second fade, and keeping the tabs in
  fullscreen.
- The channel is hidden under the player with `visibility`, and the stage is
  see-through, so anything in the channel that is slow to hide shows over the
  video. A `transition: all` did exactly that to the clicked thumbnail;
  transitions are switched off under the player for that reason.
- The media key reaches mpv whether or not the player is on screen, so it can
  start the paused video's sound while the channel is showing. Its place is
  picked up again when the player is next opened.
- One mpv log line at the first frame ("INVALID_FRAMEBUFFER_OPERATION") showed
  in the spike with no visible effect.
