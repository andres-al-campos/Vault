// A stand-in for the Rust backend, so the real frontend runs in an ordinary
// browser where it can be clicked and inspected. Loaded only by a dev server,
// and only when the page isn't inside Tauri (see src/boot.js).
//
// It answers every command the frontend invokes, from a recorded channel
// listing, and plays downloads on a timer. What it proves is the frontend:
// which button does what, which list a filter shows. The real yt-dlp path is
// covered by the Rust tests, not here.
//
// The page's query string picks the starting point:
//   ?scenario=fresh     nothing saved (the default)
//   ?scenario=library   one channel with a folder; some videos archived, one
//                       with its file missing, some marked members-only
//   &subscribed=1       the library channel is subscribed (Videos only), so
//                       launch queues what is listed above its newest archived
//   &confirm=no         the "Queue N videos?" popup is answered Cancel
//   ?scenario=corrupt   the state file can't be read
//   &failtab=shorts     the first fetch of that tab fails
//   &fail=<id>,<id>     those downloads fail on their first try
//   &bot=<n>            the next n downloads meet YouTube's bot check
//   &speed=<ms>         how long a download takes (default 1500)
//   &many=<n>           the Videos list repeated up to n entries, for a long grid
//   &retitle=1          a video's own lookup returns another title than the
//                       channel's list, as YouTube's translated titles do
//   &fixture=nasa       NASA's channel (120 recent videos, recorded 2026-10-15)
//                       instead of bill wurtz, for footage that's fine to show
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import billwurtz from "./fixtures/channel.json";
import nasa from "./fixtures/nasa.json";
// Served from here: Chrome blocks Google's copy of the avatar now and then.
import nasaAvatar from "./fixtures/nasa-avatar.jpg";
import nasaBanner from "./fixtures/nasa-banner.jpg";

const q = new URLSearchParams(location.search);
const recorded = q.get("fixture") === "nasa" ? { ...nasa, about: { ...nasa.about, avatar: nasaAvatar, banner: nasaBanner } } : billwurtz;
const scenario = q.get("scenario") || "fresh";
const speed = Number(q.get("speed") || 1500);
const failOnce = new Set((q.get("fail") || "").split(",").filter(Boolean));
let botChecks = Number(q.get("bot") || 0);
const failTabOnce = new Set((q.get("failtab") || "").split(",").filter(Boolean));

const URL_ = recorded.url ?? "https://www.youtube.com/@billwurtz";
const DIR = recorded.dir ?? "/Volumes/Archive/bill wurtz";
// The recorded listing has no members-only videos, so the library scenario
// marks a few. That flag is the one invented thing in the fixture.
// The recording predates dates in the listing: space them three weeks apart.
const videos = recorded.videos.map((v, i) => ({ ...v, timestamp: Math.floor(Date.now() / 1000) - (i * 21 + 2) * 86400 })).map((v, i) =>
  scenario === "library" && [2, 6, 10].includes(i) ? { ...v, availability: "subscriber_only" } : v);
for (let i = 0, n = videos.length; videos.length < Number(q.get("many") || 0); i++) {
  const v = videos[i % n];
  videos.push({ ...v, id: `${v.id.slice(0, 7)}${String(i).padStart(4, "0")}`, title: `${v.title} (${i + 2})` });
}
const shorts = recorded.videos.slice(40, 48).map((v) => ({ ...v, id: v.id, duration: null }));

// As the real listing returns them, flattened onto the channel.
const about = recorded.about ?? {
  avatar: "https://yt3.googleusercontent.com/ytc/AIdro_lCR9X7hGA1PPZH2S3LL1S8eBwQL0N3xaC75J3fPe8Cs6I=s900-c-k-c0x00ffffff-no-rj",
  banner: "https://yt3.googleusercontent.com/7yGjwamL8Fe_G8_k-_C9nUWvCr4HN0KOb7NVuaRZFyov1W6mTFx8i9uutSB23e8jasxVbCCzyA=w1707-fcrop64=1,00005a57ffffa5a8-k-c0xffffffff-no-nd-rj",
  followers: 5540000,
  verified: true,
};
// Archived once, since deleted from the channel: in the archive, in no listing.
// Titles of videos known only from a pasted link, which a download names its
// file by, as yt-dlp would.
const linkTitles = new Map();
const UNLISTED = { id: "gone0000001", title: "a video the channel took down" };

let state = { active: null, channels: [], cookies_browser: null };
// id -> path, or null when the archive records the id but the file is gone.
const archive = new Map();
if (scenario === "library") {
  state = {
    active: URL_, cookies_browser: null,
    channels: [{ url: URL_, title: recorded.title, uploader: recorded.uploader, video_count: videos.length, fetched_at: 0, download_dir: DIR, videos, ...about,
      subscribe: q.get("subscribed") ? { videos: true, shorts: false, streams: false } : null }],
  };
  for (const i of [1, 3, 5, 6, 9]) archive.set(videos[i].id, `${DIR}/${videos[i].title} [${videos[i].id}].mp4`);
  archive.set(videos[7].id, null);
  archive.set(UNLISTED.id, `${DIR}/${UNLISTED.title} [${UNLISTED.id}].mp4`);
}

// What the stand-in player has on: every video is 40 seconds long.
const film = { path: null, position: 0, duration: 40, paused: true, at: 0 };
function filmNow() {
  const now = Date.now();
  if (screen?.src) { film.position = screen.currentTime; film.paused = screen.paused; }
  else if (film.path && !film.paused) film.position = Math.min(film.duration, film.position + (now - film.at) / 1000);
  film.at = now;
}

// A real picture, when a clip of the video is in tests/drive/media/<id>.webm
// (fetched by hand with yt-dlp, gitignored): it plays under the page where
// mpv would draw, and the clock follows it. The time shown is the listing's,
// so a short clip of a long video reads as the long one.
const clips = import.meta.glob("./media/*.webm", { eager: true, query: "?url", import: "default" });
let screen = null;
let place = { display: "none" }; // where the page last put the picture
function clipFor(path) {
  const id = path?.match(/\[([\w-]{11})\]\.\w+$/)?.[1];
  return id && clips[`./media/${id}.webm`] ? { id, url: clips[`./media/${id}.webm`] } : null;
}
function screenEl() {
  if (screen) return screen;
  screen = document.createElement("video");
  screen.muted = true;
  screen.playsInline = true;
  Object.assign(screen.style, { position: "fixed", zIndex: "-1", objectFit: "contain", background: "#000" }, place);
  document.documentElement.appendChild(screen);
  return screen;
}

const running = new Map(); // id -> interval
const log = [];            // every command, for a drive to assert on
window.__drive = { log, archive, get state() { return state; } };

function download({ id, dir }) {
  // The real backend skips only when the finished file is already there.
  if (archive.get(id)) {
    emit("download-skipped", { id });
    emit("download-done", { id, ok: true, cancelled: false, reason: null });
    return;
  }
  const total = 24_000_000;
  let got = 0;
  running.set(id, setInterval(() => {
    got += total / 10;
    if (got < total) {
      emit("download-progress", { id, downloaded: got, total, speed: 4_100_000, eta: (total - got) / 4_100_000 });
      return;
    }
    clearInterval(running.get(id));
    running.delete(id);
    if (botChecks > 0) {
      botChecks--;
      emit("download-done", { id, ok: false, cancelled: false, reason: "YouTube bot check" });
      return;
    }
    if (failOnce.delete(id)) {
      emit("download-done", { id, ok: false, cancelled: false, reason: "Network error" });
      return;
    }
    const v = [...videos, ...shorts].find((x) => x.id === id);
    archive.set(id, `${dir}/${v?.title ?? linkTitles.get(id) ?? id} [${id}].mp4`);
    emit("download-done", { id, ok: true, cancelled: false, reason: null });
  }, speed / 10));
}

const commands = {
  load_state() {
    if (scenario === "corrupt") {
      throw "Couldn't read /Users/you/.vault/state.json: EOF while parsing a value at line 1 column 62. Vault left the file alone and won't save over it. Fix the JSON, or move the file aside to start fresh, then reopen Vault.";
    }
    return structuredClone(state);
  },
  save_state({ state: next }) { state = structuredClone(next); return null; },
  fetch_dates: () => 0,
  migrate_archives: () => ({ ran: false, channels: 0, moved: 0, unmatched: 0, meta_failed: 0 }),
  fetch_channel({ url, tab }) {
    if (/fail/i.test(url)) throw "ERROR: [youtube:tab] Sign in to confirm you're not a bot. Use --cookies-from-browser";
    if (failTabOnce.delete(tab)) throw "ERROR: unable to download webpage: timed out";
    const list = tab === "videos" ? videos : tab === "shorts" ? shorts : [];
    return { url: URL_, title: recorded.title, uploader: recorded.uploader, tab, video_count: list.length, videos: list, ...about };
  },
  // A video link: the recorded channel's videos and Shorts resolve to it;
  // any other id is a video the channel doesn't list.
  is_video_link: ({ raw }) => /youtu\.be\/|\/watch\?|\/shorts\/|\/live\/[\w-]/.test(raw),
  resolve_video({ url }) {
    if (/fail/i.test(url)) throw "ERROR: [youtube] fail: Video unavailable";
    const id = url.match(/(?:v=|youtu\.be\/|shorts\/|live\/)([\w-]{11})/)?.[1] ?? "unlisted001";
    const v = [...videos, ...shorts].find((x) => x.id === id);
    if (!v) linkTitles.set(id, "a video only its link finds");
    const title = v && q.get("retitle") ? `${v.title} (translated)` : v?.title ?? linkTitles.get(id);
    return { id, title, url: `https://www.youtube.com/watch?v=${id}`,
      channel_url: URL_, channel_urls: [URL_, "https://www.youtube.com/channel/UCq6aw03lNILzV96UvEAASfQ"],
      availability: v ? "public" : "unlisted" };
  },
  list_archive() {
    const ch = state.channels.find((c) => c.url === URL_);
    if (!ch?.download_dir) return [];
    return [...archive].map(([id, path]) => {
      const v = [...videos, ...shorts, UNLISTED].find((x) => x.id === id);
      return {
        id, title: v?.title ?? linkTitles.get(id) ?? id, duration: v?.duration ?? null, view_count: v?.view_count ?? null,
        url: v?.url ?? `https://www.youtube.com/watch?v=${id}`,
        channel: ch.title, channel_url: ch.url, dir: ch.download_dir, path,
        // One downloaded from a link alone is an unlisted video.
        availability: v ? v.availability ?? "public" : id === UNLISTED.id ? null : "unlisted",
      };
    });
  },
  download_video(args) { download(args); return null; },
  cancel_download({ id }) {
    if (!running.has(id)) return false;
    clearInterval(running.get(id));
    running.delete(id);
    emit("download-done", { id, ok: false, cancelled: true, reason: null });
    return true;
  },
  default_download_dir_str: () => "/Users/you/Downloads/Vault",
  ensure_dir: () => null,
  set_channel_dir: () => null,
  reveal_folder: () => null,
  // The player: no picture here, only a clock that runs while it's playing.
  player_load({ path, start, paused }) {
    if (/unplayable/.test(path)) throw `The file isn't there: ${path}. If it's on an external drive, connect it; otherwise press Sync and download it again.`;
    Object.assign(film, { path, position: start, paused, at: Date.now(), duration: 40 });
    const clip = clipFor(path);
    if (clip) {
      const v = screenEl();
      film.duration = [...videos, ...shorts].find((x) => x.id === clip.id)?.duration ?? 40;
      v.src = clip.url;
      v.currentTime = start;
      if (paused) v.pause(); else v.play().catch(() => {});
    } else if (screen) screen.removeAttribute("src");
    return null;
  },
  player_set({ action, value }) {
    filmNow();
    if (action === "play") film.paused = false;
    if (action === "pause") film.paused = true;
    if (action === "seek") film.position = value;
    if (action === "stop") film.path = null;
    if (screen?.src) {
      if (action === "play") screen.play().catch(() => {});
      if (action === "pause") screen.pause();
      if (action === "seek") screen.currentTime = value;
      if (action === "stop") screen.removeAttribute("src");
    }
    if (action === "mute") film.muted = !film.muted;
    return null;
  },
  player_state() {
    filmNow();
    const ended = film.position >= film.duration;
    return { position: film.position, duration: film.duration, paused: film.paused || ended, ended, path: film.path, muted: !!film.muted, ready: !!film.path };
  },
  player_place({ visible, x, top, w, h }) {
    place = { display: visible ? "block" : "none", left: `${x}px`, top: `${top}px`, width: `${w}px`, height: `${h}px` };
    if (screen) Object.assign(screen.style, place);
    return null;
  },
  // No picture to save here.
  player_still: () => null,
  player_fullscreen: ({ on }) => on ?? false,
  // The folder picker: there's no dialog to click, so it picks the suggestion.
  "plugin:dialog|open": ({ options }) => options?.defaultPath || DIR,
  // The "Queue N videos?" popup: answered yes unless ?confirm=no.
  "plugin:dialog|message": () => (q.get("confirm") === "no" ? "Cancel" : "Queue them"),
};

mockWindows("main");
mockIPC((cmd, args) => {
  log.push({ cmd, args });
  const run = commands[cmd];
  if (!run) throw `The fake backend has no "${cmd}" command. Add it to tests/drive/fake-backend.js.`;
  return run(args || {});
}, { shouldMockEvents: true });
