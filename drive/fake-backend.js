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
//   &speed=<ms>         how long a download takes (default 1500)
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import recorded from "./fixtures/channel.json";

const q = new URLSearchParams(location.search);
const scenario = q.get("scenario") || "fresh";
const speed = Number(q.get("speed") || 1500);
const failOnce = new Set((q.get("fail") || "").split(",").filter(Boolean));
const failTabOnce = new Set((q.get("failtab") || "").split(",").filter(Boolean));

const URL_ = "https://www.youtube.com/@billwurtz";
const DIR = "/Volumes/Archive/bill wurtz";
// The recorded listing has no members-only videos, so the library scenario
// marks a few. That flag is the one invented thing in the fixture.
// The recording predates dates in the listing: space them three weeks apart.
const videos = recorded.videos.map((v, i) => ({ ...v, timestamp: Math.floor(Date.now() / 1000) - (i * 21 + 2) * 86400 })).map((v, i) =>
  scenario === "library" && [2, 6, 10].includes(i) ? { ...v, availability: "subscriber_only" } : v);
const shorts = recorded.videos.slice(40, 48).map((v) => ({ ...v, id: v.id, duration: null }));

// As the real listing returns them, flattened onto the channel.
const about = {
  avatar: "https://yt3.googleusercontent.com/ytc/AIdro_lCR9X7hGA1PPZH2S3LL1S8eBwQL0N3xaC75J3fPe8Cs6I=s900-c-k-c0x00ffffff-no-rj",
  banner: "https://yt3.googleusercontent.com/7yGjwamL8Fe_G8_k-_C9nUWvCr4HN0KOb7NVuaRZFyov1W6mTFx8i9uutSB23e8jasxVbCCzyA=w1707-fcrop64=1,00005a57ffffa5a8-k-c0xffffffff-no-nd-rj",
  followers: 5540000,
  verified: true,
};
// Archived once, since deleted from the channel: in the archive, in no listing.
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

const running = new Map(); // id -> interval
const log = [];            // every command, for a drive to assert on
window.__drive = { log, archive, get state() { return state; } };

function download({ id, dir, force }) {
  if (archive.has(id) && !force) {
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
    if (failOnce.delete(id)) {
      emit("download-done", { id, ok: false, cancelled: false, reason: "Network error" });
      return;
    }
    const v = [...videos, ...shorts].find((x) => x.id === id);
    archive.set(id, `${dir}/${v?.title ?? id} [${id}].mp4`);
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
  migrate_archives: () => ({ ran: false, channels: 0, moved: 0, unmatched: 0, meta_failed: 0 }),
  fetch_channel({ url, tab }) {
    if (/fail/i.test(url)) throw "ERROR: [youtube:tab] Sign in to confirm you're not a bot. Use --cookies-from-browser";
    if (failTabOnce.delete(tab)) throw "ERROR: unable to download webpage: timed out";
    const list = tab === "videos" ? videos : tab === "shorts" ? shorts : [];
    return { url: URL_, title: recorded.title, uploader: recorded.uploader, tab, video_count: list.length, videos: list, ...about };
  },
  list_archive() {
    const ch = state.channels.find((c) => c.url === URL_);
    if (!ch?.download_dir) return [];
    return [...archive].map(([id, path]) => {
      const v = [...videos, ...shorts, UNLISTED].find((x) => x.id === id);
      return {
        id, title: v?.title ?? id, duration: v?.duration ?? null, view_count: v?.view_count ?? null,
        url: v?.url ?? `https://www.youtube.com/watch?v=${id}`,
        channel: ch.title, channel_url: ch.url, dir: ch.download_dir, path,
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
  open_file: () => null,
  // The folder picker: there's no dialog to click, so it picks the suggestion.
  "plugin:dialog|open": ({ options }) => options?.defaultPath || DIR,
  // The "Queue N videos?" popup: answered yes unless ?confirm=no.
  "plugin:dialog|message": () => (q.get("confirm") === "no" ? "Cancel" : "Queue them"),
};

mockWindows("main");
mockIPC((cmd, args) => {
  log.push({ cmd, args });
  const run = commands[cmd];
  if (!run) throw `The fake backend has no "${cmd}" command. Add it to drive/fake-backend.js.`;
  return run(args || {});
}, { shouldMockEvents: true });
