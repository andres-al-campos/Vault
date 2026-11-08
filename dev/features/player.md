# Player

Updated 2026-11-07 23:08, at 0.1.0.

Archived videos play inside Vault. Each one opens in a tab that keeps its
place, so several can be open at once and picked up later, the way YouTube
videos sit in browser tabs through a day.

## Sub-features

- Play on an archived card opens the video in a tab and shows the player.
  ⌘-click opens the tab without leaving the channel. A finished download's
  thumbnail or title in the Downloads sidebar does the same.
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
  mutes by moving the volume bar to 0, and unmuting puts it back where it was, Esc goes back to the channel.
- Fullscreen: the button at the right of the controls, or F. The window goes
  fullscreen and the tabs and controls lie over
  the picture. Each bar comes up only when the pointer moves within 200px of
  its edge, fading and sliding in from that edge, and goes the same way 2.5
  seconds after the pointer was last in its zone. Moving across the
  middle of the picture shows nothing. Of the keys, only play/pause brings
  them up. They stay while the video is paused or the pointer is on them. F or Esc
  leaves; a second Esc goes back to the channel. A window that was already
  fullscreen stays so on the way out.
- Pointing along the seek bar shows the time a click there would land on.
  Pointing ahead of the handle also fills the bar from the handle to the
  pointer in grey: the stretch a click there would skip.
- Starting or stopping playback blinks a play or pause mark in the middle of
  the picture, muting or unmuting blinks a speaker the same way, and a skip
  from the keys blinks how far it went with a chevron on that side
  ("‹ 5s", "10s ›").
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

Play on any archived card (the thumbnail or the button under it), the thumbnail
or title of a finished download in the sidebar, or "Open videos" in the top bar.

## Driving it

`./build.sh check` (~15s): `src/player.test.js` covers the tab rules: one tab
per video, background opens, which tab takes over on close, per-tab resume,
starting over near the end, and reading saved tabs back, damaged ones included.

By click against the stand-in (`?scenario=library`): its player is a clock
with no picture, every video 40 seconds long. It shows opening, switching,
resume after a reload, replay at the end, Space, Esc and closing.

With a clip of the video in `tests/drive/media/<id>.webm` (gitignored; fetch the
first seconds with `yt-dlp --download-sections "*0-22"` and convert to VP9,
which every Chromium plays), the stand-in plays it in a `<video>` under the
page where mpv would draw, and the clock follows it. Used with
`&fixture=nasa` and `Ke6XX8FHOHM` for the portfolio demo, 2026-10-15: the
cover faded into the picture, it played at speed, and a click paused it.

By hand only: mpv's own picture (scaling, colour, the frame saved on
leaving). Chrome's `<video>` stands in for it above.

Last driven: 2026-10-12, stand-in. Two tabs opened, one in the background;
switching away and back reloaded the first at 1.2s; after a reload the active
tab loaded at 18.8s; seeking to the end showed "Play again", which restarted
it from 0.

## Calls

Look-and-feel calls made for this feature, with the reason given at the
time.

- 2026-10-12 16:52, 0.1.0. The player's tabs look and feel like a browser's tab strip
  (Chrome's was the reference, without the site icon): a raised fill for the
  one on screen, hairlines between the rest, close on hover, the full title
  on a card below. Asked for by that look; the filters keep their own.
  No hairline beside the tab on screen: it looked better without. So that
  tab's edge can mark where its neighbour ends, the bar leaves little space
  above and below it.
- 2026-10-12 18:24, 0.1.0. The player's top and bottom bars are the same height, and
  short: more of the window goes to the video.
- 2026-10-12 19:34, 0.1.0. The video is shown as sRGB, not pushed to the screen's full
  gamut: the more saturated picture looked unrealistic.
- 2026-10-12 19:47, 0.1.0. The seek bar is YouTube's red, as an extra touch of YouTube. The
  same red marks progress on a tab's hover card.
- 2026-10-13 15:33, 0.1.0. Fullscreen is the button or F. No double-click: two ways in
  are enough.
- 2026-10-13 23:12, 0.1.0. Pointing ahead on the seek bar shows a grey bar up to the
  pointer, as most players do, in the volume bar's grey.

- 2026-10-28 02:23, 0.1.0. In fullscreen each bar comes up only when the pointer heads for its
  edge, not on any movement: the bars came up at a millimetre's movement. Only
  play/pause brings them up from the keyboard. They fade and slide a little
  from their edge rather than appearing at once. A still pointer in a zone
  lets the bar go, as most players do; only the bar itself holds it.
  Applies *Motion spends attention*.
- 2026-10-28 02:23, 0.1.0. Mute moves the volume bar to 0 and unmute puts back the level from
  before, so the bar shows what you hear.
- 2026-10-28 02:23, 0.1.0. Play and the speaker are drawn to look the same size as the fullscreen
  corners, which looked right at their size.

- 2026-10-29 08:59, 0.1.0. A skip's mark is one chevron beside the seconds, not "+5s": plus
  and minus felt like maths, not moving through the video. One chevron, as
  YouTube's player has it (read from its markup, 2026-10-29); two read as
  fast-forward.

## Gotchas

- The clock redraws the bar four times a second. Replacing a button's icon
  each time made clicks on play, mute and fullscreen sometimes do nothing
  (pressed on one icon, released on its copy). `tests/steady-buttons.sh`
  keeps renderTransport to `setIcon`, which swaps only a changed icon.

- The seek bar's grey bar: its 0.12-second fade, and that nothing shows when
  pointing behind the handle, are an agent's guesses. Seen in a mockup on
  2026-10-13, not in the app.
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
- The media key works whether or not the player is on screen, so it can
  start the paused video's sound while the channel is showing. Its place is
  picked up again when the player is next opened. mpv receives the key, but
  its keybinds send it on to the page as `player-play-key`, which presses
  Play: mpv's own toggle did nothing once a video had been unloaded.
- The saved frames are JPEGs in the system's temporary folder
  (`vault-stills/`), one per tab, replaced on each save. A closed tab's file
  is left for the system to clear. Each is shrunk to the stage's width just
  after it's saved: every save has a new name, so the page decodes a new
  picture each time, and at the video's own size that's about 33 MB for 4K.
- There is one player. Switching tabs unloads one file and loads the other,
  so the cover hides a real load; two videos are never live at once.
- A video that hasn't played for 5 minutes (paused or finished; going back
  to the channel pauses it) is unloaded: one loaded video held about 700 MB in a test on an
  M1 Pro, most of it software decoding, since downloads prefer AV1 and that
  chip has no AV1 hardware decoder. On screen its frame goes up as the
  cover first, and Play, the play key or coming back loads it again.
- Play presses while a video is starting (from the press until mpv reports
  its first frame, at most 10 seconds) are ignored, and the first one
  flashes the play icon. Each press used to start the load over, so pressing
  Space a few times on an unloaded video got nowhere until the presses
  stopped.
- Two failures show on the stage. A file that isn't there is refused before
  mpv sees it (`player_load`). One mpv takes and then can't read, damaged or
  on a drive that went away mid-video, comes back as mpv's end-file event
  (`player-file-failed`, from the event thread in `player.rs`); the frame
  stays up under the message, and Play tries the file again.
- One mpv log line at the first frame ("INVALID_FRAMEBUFFER_OPERATION") showed
  in the spike with no visible effect.
