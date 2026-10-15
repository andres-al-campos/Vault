import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, ask } from "@tauri-apps/plugin-dialog";
import { createQueue, lineAction } from "./queue.js";
import { createTabs, resumeAt, progress } from "./player.js";
import { isMembers, archiveState, view, downloadable, withUnlisted, batchSizes, batchOf, windowOf, ASK_ABOVE, newVideos, ago } from "./library.js";

/* ---------------- theme ---------------- */
const root = document.documentElement;
const savedTheme = localStorage.getItem("vault-theme");
if (savedTheme) root.setAttribute("data-theme", savedTheme);
document.getElementById("theme").addEventListener("click", () => {
  const next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
  root.setAttribute("data-theme", next);
  localStorage.setItem("vault-theme", next);
});

// The download jobs and their order — see queue.js. What hadn't finished when
// Vault last closed comes back, paused if it was paused.
const QUEUE_KEY = "vault-queue";
const queue = createQueue((() => { try { return JSON.parse(localStorage.getItem(QUEUE_KEY)); } catch { return null; } })());

/* ---------------- downloads panel ---------------- */
// Vault opens on what you've kept, so the panel starts hidden unless
// something is still in line from last time. After that it stays where the
// button puts it.
const appEl = document.querySelector(".app");
const sideBtn = document.getElementById("side-toggle");
let sideShown = queue.jobs.size > 0;
localStorage.removeItem("vault-side"); // where the panel's state was kept until 2026-10-14
let unfinished = 0;
function renderSide() {
  appEl.classList.toggle("side-off", !sideShown);
  sideBtn.setAttribute("aria-pressed", String(sideShown));
  sideBtn.title = sideShown ? "Hide downloads" : "Show downloads";
  // Hidden, the button says how many downloads are still to finish.
  const n = document.getElementById("side-n");
  n.hidden = sideShown || unfinished === 0;
  n.textContent = unfinished;
}
// A panel that starts hidden is hidden from the first frame, without sliding shut.
appEl.style.transition = "none";
renderSide();
void appEl.offsetWidth;
appEl.style.transition = "";
sideBtn.addEventListener("click", () => {
  sideShown = !sideShown;
  renderSide();
});

/* ---------------- helpers ---------------- */
const $ = (id) => document.getElementById(id);

function fmtDur(sec) {
  if (!sec && sec !== 0) return "";
  sec = Math.round(sec);
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  const pad = (n) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
function fmtViews(n) {
  if (n == null) return "";
  if (n >= 1e9) return (n / 1e9).toFixed(1).replace(/\.0$/, "") + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, "") + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1).replace(/\.0$/, "") + "K";
  return String(n);
}
function fmtBytes(b) {
  if (!b || b < 0) return "—";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return b.toFixed(b < 10 && i > 0 ? 1 : 0) + " " + u[i];
}
const thumb = (id) => `https://i.ytimg.com/vi/${id}/mqdefault.jpg`;
function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}
function safeName(s) { return String(s).replace(/[/\\:]/g, "-").trim() || "channel"; }
// A remembered choice, or the default when what's stored isn't one of the options.
const remembered = (key, options) => {
  const saved = localStorage.getItem(key);
  return options.includes(saved) ? saved : options[0];
};

/* ---------------- state ---------------- */
let state = { active: null, channels: [] };
const saveQueue = () => { try { localStorage.setItem(QUEUE_KEY, JSON.stringify(queue.snapshot())); } catch { /* a full store only costs the restore */ } };
let queueSave = null;
// Progress reports arrive several times a second; the line itself changes rarely.
const saveQueueSoon = () => { queueSave ??= setTimeout(() => { queueSave = null; saveQueue(); }, 400); };
window.addEventListener("beforeunload", saveQueue);

// yt-dlp reads cookies straight from the chosen browser's profile, so
// members-only videos and the bot check are handled without ever handling
// the cookie file itself.
const BROWSERS = ["chrome", "safari", "firefox", "floorp","edge", "brave", "chromium"];

// Shorts and the live tab are fetched on demand and kept in memory only —
// the Videos tab is the one that persists.
const tabCache = new Map(); // channelUrl -> { shorts: [...], live: [...] }
// Why a tab's list couldn't be fetched, by `${channelUrl}|${tab}`. Kept apart
// from tabCache so a failure never reads as "this channel has no shorts".
const tabErrors = new Map();
const tabLoading = new Set();
const tabId = (ch, tab) => `${ch.url}|${tab}`;
const isLazyTab = (tab) => tab === "shorts" || tab === "streams";

// Every channel's archive entries, read from the archive files in the channel
// folders. Rebuilt on demand (boot, after a download finishes) rather than
// persisted — the files are the source of truth.
let archive = [];
// The entries of the channel on screen, by video id. An id counts as archived
// only for the channel whose folder records it, so two channels that share a
// video don't mark each other's copy.
let archived = new Map();

const TABS = { videos: "Videos", shorts: "Shorts", streams: "Live" };
const SORTS = { newest: "Newest", oldest: "Oldest", popular: "Most popular", shortest: "Shortest", longest: "Longest" };
const ACCESS = { all: "All", public: "Public", members: "Members" };
const HAVE = { all: "All", archived: "Archived", not: "Not archived" };

let activeTab = remembered("vault-tab", Object.keys(TABS));
const filters = {
  sort: remembered("vault-sort", Object.keys(SORTS)),
  access: remembered("vault-access", Object.keys(ACCESS)),
  have: remembered("vault-have", Object.keys(HAVE)),
};

const activeChannel = () => state.channels.find((c) => c.url === state.active) || null;

async function loadArchive() {
  try {
    archive = await invoke("list_archive");
  } catch (e) {
    archive = [];
    console.error("Could not read the archive:", e);
  }
  rebuildArchived();
}

// Ownership comes from the archive files, so a video that has dropped off the
// channel's fetched list (deleted on YouTube) is still here.
const scopedArchive = () => archive.filter((e) => e.channel_url === state.active);
function rebuildArchived() {
  archived = new Map(scopedArchive().map((e) => [e.id, e]));
}

// Set when the state file exists but couldn't be read. Nothing is saved while
// it is: the in-memory state is empty, and writing it would replace the real
// channel list. The backend refuses the write too.
let stateError = null;

async function persist() {
  if (stateError) return;
  try { await invoke("save_state", { state }); }
  catch (e) { console.error("Could not save state:", e); }
}

/* ---------------- channel selector ---------------- */
const selBtn = $("chansel-btn");
const selMenu = $("chansel-menu");

function renderSelector() {
  const ch = activeChannel();
  selBtn.disabled = state.channels.length === 0;
  $("chansel-lbl").textContent = ch ? ch.title : (state.channels.length ? "Choose a channel" : "No channels yet");
  if (state.channels.length === 0) {
    selMenu.innerHTML = `<div class="chansel-empty">Channels you fetch will show up here.</div>`;
    return;
  }
  selMenu.innerHTML = state.channels.map((c) => `
    <div class="chansel-item ${c.url === state.active ? "active" : ""}" data-url="${esc(c.url)}">
      <span class="nm">${esc(c.title)}</span>
      <span class="ct">${c.video_count}</span>
      <button class="rm" data-rm="${esc(c.url)}" title="Forget this channel">✕</button>
    </div>`).join("");
}

selBtn.addEventListener("click", (e) => { e.stopPropagation(); selMenu.classList.toggle("open"); });
document.addEventListener("click", () => selMenu.classList.remove("open"));

selMenu.addEventListener("click", async (e) => {
  const rm = e.target.closest("[data-rm]");
  if (rm) {
    e.stopPropagation();
    const url = rm.dataset.rm;
    state.channels = state.channels.filter((c) => c.url !== url);
    tabCache.delete(url);
    if (state.active === url) state.active = state.channels[0]?.url ?? null;
    await persist();
    renderSelector();
    showActive();
    return;
  }
  const item = e.target.closest("[data-url]");
  if (!item) return;
  state.active = item.dataset.url;
  activeTab = "videos";
  await persist();
  renderSelector();
  showActive();
  selMenu.classList.remove("open");
});

/* ---------------- fetch ---------------- */
const emptyEl = $("empty");
const gridEl = $("grid");
const headEl = $("chanhead");
const controlsEl = $("controls");
// What the page says with no channel: written once, in index.html.
const welcome = emptyEl.innerHTML;
// The faded wordmark over every message in the main column; index.html's
// own copy is the welcome above.
const MARK = emptyEl.querySelector(".mark").outerHTML;

function setEmpty(html) {
  emptyEl.innerHTML = html;
  emptyEl.style.display = html ? "flex" : "none";
  gridEl.style.display = html ? "none" : "grid";
  controlsEl.innerHTML = "";
  headEl.innerHTML = "";
}

async function doFetch(raw, { quiet = false } = {}) {
  raw = (raw || "").trim();
  if (!raw) return;
  if (!quiet && await invoke("is_video_link", { raw }).catch(() => false)) return openVideoLink(raw);
  if (!quiet) setEmpty(`${MARK}<h2>Fetching…</h2><p>Reading the channel's video list.</p><div class="spin"></div>`);
  $("fetch").disabled = true;
  try {
    const info = await invoke("fetch_channel", { url: raw, tab: "videos", cookies: state.cookies_browser ?? null });
    if (!info.videos || !info.videos.length) {
      if (!quiet) setEmpty(`${MARK}<h2>No videos found</h2><p>That returned nothing. Check the URL or @handle and try again.</p>`);
      else alert(`Couldn't refresh ${raw}.\n\nYouTube returned no videos. The saved list is unchanged — try again in a moment.`);
      return;
    }
    let ch = state.channels.find((c) => c.url === info.url);
    if (!ch) {
      ch = { url: info.url, title: info.title, uploader: info.uploader, video_count: 0, fetched_at: 0, download_dir: null, videos: [] };
      state.channels.unshift(ch);
    }
    const before = takeListing(ch, info);
    state.active = info.url;
    tabCache.delete(info.url); // tab caches are stale after a re-fetch
    for (const k of Array.from(tabErrors.keys())) if (k.startsWith(`${info.url}|`)) tabErrors.delete(k);
    activeTab = "videos";
    await persist();
    // A fetch is also when the folder gets looked at again, so Sync picks up
    // files added, moved or deleted outside the app, not only new uploads.
    await loadArchive();
    renderSelector();
    showActive();
    await queueNew(ch, before);
    fillDates(ch);
  } catch (e) {
    if (!quiet) setEmpty(`${MARK}<h2>Couldn't fetch that</h2><p>${esc(String(e))}</p>`);
    // Sync keeps the list on screen, so its failure has nowhere to show. Say
    // it rather than leave the click looking ignored.
    else alert(`Couldn't refresh ${raw}.\n\n${e}\n\nThe saved list is unchanged.`);
  } finally {
    $("fetch").disabled = false;
  }
}

// A link to one video opens its channel, adding it first if it's new, with
// the video's title in the search box and both filters at All, so its card is
// what's on screen. Nothing is downloaded: the card's button does that.
let pasted = null; // the video a link asked for, while its title is searched
async function openVideoLink(raw) {
  setEmpty(`${MARK}<h2>Finding the video’s channel…</h2><p>Asking YouTube whose video this is.</p><div class="spin"></div>`);
  $("fetch").disabled = true;
  let ref;
  try {
    ref = await invoke("resolve_video", { url: raw, cookies: state.cookies_browser ?? null });
  } catch (e) {
    setEmpty(`${MARK}<h2>Couldn't open that video</h2><p>${esc(String(e))}</p>`);
    return;
  } finally {
    $("fetch").disabled = false;
  }
  const known = () => state.channels.find((c) => ref.channel_urls.includes(c.url));
  let ch = known();
  if (!ch) {
    await doFetch(ref.channel_url);
    ch = known();
    if (!ch || activeChannel() !== ch) return; // the fetch failed and said why
  } else {
    state.active = ch.url;
    await persist();
    renderSelector();
  }
  setEmpty(`${MARK}<h2>Finding the video…</h2><p>Looking through the channel’s tabs.</p><div class="spin"></div>`);
  let tab = await tabOf(ch, ref.id);
  // A saved channel may not list a video newer than its last Sync.
  if (!tab && ch.fetched_at < Date.now() / 1000 - 60) {
    await doFetch(ch.url, { quiet: true });
    tab = await tabOf(ch, ref.id);
  }
  activeTab = tab ?? "videos";
  localStorage.setItem("vault-tab", activeTab);
  clearFilters();
  queryFor = ch.url;
  // The title the channel's list gives, which is the one its card shows. A
  // video looked up on its own can come back under another, a translation
  // say, and searching for that found nothing.
  setQuery((tab && tabVideos(ch, tab).find((v) => v.id === ref.id)?.title) || ref.title);
  pasted = { ...ref, channel: ch.url, listed: !!tab };
  setUrl("");
  showActive();
}

// The tab a video is on, fetching Shorts and Live if they haven't been, or
// null when the channel doesn't list it anywhere.
async function tabOf(ch, id) {
  for (const tab of Object.keys(TABS)) {
    if (isLazyTab(tab) && !tabVideos(ch, tab)) await loadTab(ch, tab);
    if (tabVideos(ch, tab)?.some((v) => v.id === id)) return tab;
  }
  return null;
}

// Puts a fresh Videos listing on a channel. Returns the ids it had before, or
// null when it had no list, which is what a subscription with nothing
// archived yet measures "new" against.
function takeListing(ch, info) {
  const before = ch.videos.length ? new Set(ch.videos.map((v) => v.id)) : null;
  ch.title = info.title;
  ch.uploader = info.uploader;
  ch.video_count = info.video_count;
  ch.videos = info.videos;
  // What the header draws. Saved with the channel so it's there on launch.
  ch.avatar = info.avatar ?? null;
  ch.banner = info.banner ?? null;
  ch.followers = info.followers ?? null;
  ch.verified = !!info.verified;
  ch.fetched_at = Math.floor(Date.now() / 1000);
  return before;
}

// Looks up the upload date of archived videos that have none: ones from
// before dates were recorded, or copied in by hand. One request per video, so
// it runs in the background and the cards update when it lands.
const datesRunning = new Set();
async function fillDates(ch) {
  const dir = ch.download_dir;
  if (!dir || datesRunning.has(dir)) return;
  datesRunning.add(dir);
  try {
    const added = await invoke("fetch_dates", { dir, cookies: state.cookies_browser ?? null });
    if (added) {
      await loadArchive();
      if (activeChannel() === ch) renderList();
    }
  } catch (e) {
    console.error(`Couldn't look up upload dates for ${ch.url}:`, e);
  } finally {
    datesRunning.delete(dir);
  }
}

/* ---------------- subscriptions ---------------- */
// Queues a subscribed channel's new uploads, tab by tab. Called after its
// Videos list has just been refreshed; Shorts and Live are fetched here when
// the subscription includes them. Returns how many were queued.
async function queueNew(ch, before) {
  const sub = ch.subscribe;
  if (!sub || !ch.download_dir) return 0;
  const mine = new Map(archive.filter((e) => e.channel_url === ch.url).map((e) => [e.id, e]));
  const lists = [];
  if (sub.videos) lists.push([ch.videos, before]);
  for (const tab of ["shorts", "streams"]) {
    if (!sub[tab]) continue;
    await loadTab(ch, tab);
    const list = (tabCache.get(ch.url) || {})[tabKey(tab)];
    if (list) lists.push([list, null]);
  }
  let queued = 0;
  for (const [list, was] of lists) {
    // Members-only videos come along when cookies are set, and wait for a
    // later refresh when they aren't.
    const { jobs } = downloadable(newVideos(list, mine, was), mine, { cookies: !!state.cookies_browser, holds: queue.holds });
    for (const { video } of jobs) {
      if (queue.add({ id: video.id, title: video.title, url: video.url, dir: ch.download_dir })) queued++;
    }
  }
  if (queued) {
    renderQueue();
    if (activeChannel() === ch) renderList();
    pump();
  }
  return queued;
}

// At launch: refresh every subscribed channel and queue what's new. One at a
// time, in the background; the saved lists are on screen meanwhile.
async function checkSubscriptions() {
  for (const ch of state.channels.filter((c) => c.subscribe)) {
    try {
      const info = await invoke("fetch_channel", { url: ch.url, tab: "videos", cookies: state.cookies_browser ?? null });
      if (!info.videos?.length) continue;
      const before = takeListing(ch, info);
      await persist();
      if (activeChannel() === ch) { renderHeader(ch); renderList(); }
      await queueNew(ch, before);
    } catch (e) {
      // The saved list stays as it was; Sync on the channel shows the reason.
      console.error(`Subscription check failed for ${ch.url}:`, e);
    }
  }
  renderSelector();
}

async function setSubscription(ch, sub) {
  if (sub && !(await ensureDir(ch))) return; // new uploads need somewhere to go
  ch.subscribe = sub;
  await persist();
  if (activeChannel() === ch) renderHeader(ch);
}

function showActive() {
  const ch = activeChannel();
  if (!ch) {
    setEmpty(welcome);
    $("sync").disabled = true;
    return;
  }
  rebuildArchived();
  $("sync").disabled = $("sync").classList.contains("syncing"); // greyed out until a Sync ends
  // The field is only for adding a channel; Sync refreshes the one on screen.
  setUrl("");
  renderHeader(ch);
  renderList();
  // The tab is remembered across launches but Shorts and Live aren't saved,
  // so a remembered tab can arrive here with nothing fetched yet.
  if (isLazyTab(activeTab) && !tabVideos(ch, activeTab) && !tabErrors.has(tabId(ch, activeTab))) {
    loadTab(ch, activeTab).then(() => {
      if (activeChannel() === ch) renderList();
    });
  }
}

const FOLDER_ICON = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>`;

// Laid out like the channel's page on YouTube: banner, round avatar, name,
// handle and counts. Channels saved before the art was fetched have none until
// their next Sync, and fall back to the initial.
function renderHeader(ch) {
  const initial = (ch.title || "?").trim().charAt(0).toUpperCase();
  const sub = ch.subscribe;
  const handle = ch.url.split("/").pop();
  const meta = [
    handle.startsWith("@") && esc(handle),
    ch.followers != null && `${fmtViews(ch.followers)} subscribers`,
    `${ch.video_count} videos`,
  ].filter(Boolean).join(" · ");
  const dir = ch.download_dir
    ? `<div class="dirmeta">
         <a class="path" id="open-dir" title="Open this folder">${esc(ch.download_dir)}</a>
         <button class="mini" id="change-dir" title="Change the download folder">${FOLDER_ICON}</button>
       </div>`
    : `<div class="dirmeta">
         <span>No download folder yet — you'll be asked when you download the first video.</span>
         <button class="mini accent" id="change-dir" title="Choose the download folder">${FOLDER_ICON}</button>
       </div>`;

  const wasOpen = openMenus(headEl);
  headEl.innerHTML = `
    ${ch.banner ? `<img class="banner" src="${esc(ch.banner)}" alt="">` : ""}
    <div class="chan">
      <div class="avatar">${ch.avatar ? `<img src="${esc(ch.avatar)}" alt="">` : esc(initial)}</div>
      <div class="chan-info">
        <div class="chan-title">
          <h1>${esc(ch.title)}${ch.verified ? `<span class="tick" title="Verified">✓</span>` : ""}</h1>
      <div class="subscribe">
        <button class="btn ${sub ? "btn-icon btn-labelled" : "btn-low"}" id="sub-btn" title="Downloads new videos that haven't been archived yet, each time Vault opens or you press Sync.">${sub ? "✓ Subscribed" : "Subscribe"}</button>
        <div class="dl-menu sub-menu" id="sub-menu">
          <div class="cookie-hd">Download new uploads from</div>
          ${Object.entries(TABS).map(([tab, label]) =>
            `<label class="sub-opt"><input type="checkbox" data-sub="${tab}"${(sub ? sub[tab] : tab === "videos") ? " checked" : ""}> ${label}</label>`).join("")}
          <div class="cookie-note">New means listed above the newest video you've archived. Members-only videos are included when cookies are set.</div>
          <button class="btn btn-primary" id="sub-go">${sub ? "Unsubscribe" : "Subscribe"}</button>
        </div>
      </div>
        </div>
        <div class="chan-meta">${meta}</div>
        ${dir}
      </div>
    </div>`;

  const picked = () => Object.fromEntries(
    [...headEl.querySelectorAll("[data-sub]")].map((b) => [b.dataset.sub, b.checked]));
  $("sub-btn").addEventListener("click", (e) => { e.stopPropagation(); $("sub-menu").classList.toggle("open"); });
  $("sub-menu").addEventListener("click", (e) => e.stopPropagation());
  fadeShut(wasOpen);
  // Once subscribed, a tick takes effect as it's made.
  headEl.querySelectorAll("[data-sub]").forEach((b) => b.addEventListener("change", () => {
    if (ch.subscribe) { ch.subscribe = picked(); persist(); }
  }));
  $("sub-go").addEventListener("click", () => setSubscription(ch, ch.subscribe ? null : picked()));

  // Art that fails to load steps aside: the initial for the avatar, nothing
  // for the banner. A broken-image box is worse than either.
  headEl.querySelector(".avatar img")?.addEventListener("error", (e) => { e.target.parentElement.textContent = initial; });
  headEl.querySelector(".banner")?.addEventListener("error", (e) => e.target.remove());
  $("change-dir").addEventListener("click", () => changeDir(ch));
  $("open-dir")?.addEventListener("click", () => invoke("reveal_folder", { dir: ch.download_dir }).catch(() => {}));
}

/* ---------------- tabs / filters / sort ---------------- */
const tabKey = (tab) => (tab === "streams" ? "live" : "shorts");

// A tab's videos, or null when a lazy tab hasn't been fetched yet. The Videos
// tab also carries archived videos the channel no longer lists: there is no
// separate Archive tab, so this is where they stay visible.
function tabVideos(ch, tab) {
  const cached = tabCache.get(ch.url) || {};
  if (tab !== "videos") return cached[tabKey(tab)] || null;
  const known = new Set([...ch.videos, ...(cached.shorts || []), ...(cached.live || [])].map((v) => v.id));
  return withUnlisted(ch.videos, scopedArchive(), known);
}

// The list on screen and the numbers behind the filters, or null while the
// tab has nothing to filter yet.
function currentView() {
  const ch = activeChannel();
  searchScope(ch);
  const list = ch && tabVideos(ch, activeTab);
  return list ? view(list, { ...filters, query }, archived) : null;
}

// The search box. It is made once and kept, not redrawn with the rest of its
// row: the row is rebuilt on every change, so a box inside it would lose its
// text cursor at every key.
let query = "";
let queryFor = null; // the channel the search was typed on
const findEl = document.createElement("label");
findEl.className = "find";
findEl.innerHTML = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/></svg>
  <input id="find" placeholder="Search this channel" spellcheck="false" autocomplete="off">
  <span class="find-n" id="find-n"></span>
  <button class="find-x" id="find-x" title="Clear (Esc)" hidden><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"><path d="M6 6l12 12M18 6L6 18"/></svg></button>`;
const findInput = findEl.querySelector("input");
const findX = findEl.querySelector(".find-x");
const findN = findEl.querySelector(".find-n");
function setQuery(text) {
  query = text;
  findInput.value = text;
  findX.hidden = !text;
}
// A search belongs to the channel it was typed on.
function searchScope(ch) {
  if ((ch?.url ?? null) === queryFor) return;
  queryFor = ch?.url ?? null;
  setQuery("");
}
findInput.addEventListener("input", () => { setQuery(findInput.value); renderList(); });
findX.addEventListener("click", (e) => { e.preventDefault(); setQuery(""); renderList(); findInput.focus(); });
// Esc clears the box, and a second Esc leaves it.
findInput.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  e.stopPropagation();
  if (query) { setQuery(""); renderList(); } else findInput.blur();
});
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey && e.key === "f") || watching() || !activeChannel()) return;
  e.preventDefault();
  findInput.focus();
  findInput.select();
});
let findAllWas = false; // so the Show all button fades in only as it appears

// Controls and grid always redraw together: every number in the controls is
// derived from the same list the grid shows.
function renderList() {
  const now = currentView();
  // Rebuilding empties the page for a moment, which would drop the scroll
  // position to the top: a filter or a download click keeps you where you were.
  const main = document.querySelector(".main");
  const at = main.scrollTop;
  renderControls(now);
  renderGrid(now);
  main.scrollTop = at;
  drawWindow(); // for where the scroll has just been put back
}

// Nothing on the page changes place at once: it moves or fades there.
//
// The mark under a row's chosen option (a filter's fill, the tabs' underline)
// is its own element, placed over `on`, since the row itself is redrawn on
// every change. It slides there from `from`: where the last mark was when
// the row was redrawn (see whereIs), which may be partway through a slide.
// A mark with nowhere it was fades in; one with nowhere to go fades out
// where it was. `slot` names which mark it is when a row has more than one.
const whereIs = (mark) => {
  if (!mark) return null;
  const css = getComputedStyle(mark);
  return { width: css.width, transform: css.transform };
};
function slideMark(row, cls, on, from, slot = "") {
  const mark = document.createElement("i");
  mark.className = cls;
  mark.dataset.slot = slot;
  if (!on) {
    if (!from) return;
    Object.assign(mark.style, from);
    mark.classList.add("leaving");
    mark.addEventListener("animationend", () => mark.remove());
    row.prepend(mark);
    return;
  }
  if (!from) mark.classList.add("coming");
  // Measured before the mark goes in and before any of it is styled:
  // reading a size lays the page out, and a mark laid out half-placed
  // starts its slide from the row's left edge instead of the last choice.
  const end = { width: `${on.offsetWidth}px`, transform: `translateX(${on.offsetLeft}px)` };
  const x = (t) => new DOMMatrix(t).m41;
  const start = from && (Math.abs(x(from.transform) - x(end.transform)) > 0.5 || Math.abs(parseFloat(from.width) - parseFloat(end.width)) > 0.5) ? from : null;
  Object.assign(mark.style, start ?? end);
  row.prepend(mark);
  // The slide starts two frames on, once the redrawn page is on screen.
  // Started here, its clock runs while the page is still being laid out and
  // can be over before the first frame shows.
  if (start) {
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!mark.isConnected) return;
      Object.assign(mark.style, end);
      on.classList.add("arriving");
    }));
  }
}

// The numbers on the filters count from what they read to what they read
// now, over the fill's slide, instead of changing at once. A number with
// more or fewer digits eases to its new width too, so the fills, measured
// against the new widths, slide alongside the options they belong to. `was`
// is what each number read before the redraw, or null for a new channel or
// tab, whose numbers just appear.
function rollNumbers(row, was) {
  if (!row || !was) return;
  const rolls = [];
  for (const b of row.querySelectorAll("[data-pick]")) {
    const n = b.querySelector(".n");
    const to = Number(n.textContent), from = was[b.dataset.pick];
    if (from == null || from === to) continue;
    // Put back as it read, after the fills were measured on the new layout.
    n.style.width = `${String(from).length}ch`;
    n.textContent = from;
    rolls.push({ n, from, to });
  }
  if (!rolls.length) return;
  // Two frames on, like the fills' slide, so both start together.
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const t0 = performance.now();
    for (const r of rolls) r.n.style.width = `${String(r.to).length}ch`;
    const step = (t) => {
      const k = Math.min(1, (t - t0) / 180);
      const eased = 1 - (1 - k) ** 3; // out: quick to leave, slow to settle
      for (const r of rolls) r.n.textContent = Math.round(r.from + (r.to - r.from) * eased);
      if (k < 1) requestAnimationFrame(step);
      else for (const r of rolls) r.n.style.width = "";
    };
    requestAnimationFrame(step);
  }));
}

// A menu whose row was picked is redrawn along with everything around it,
// which would make it vanish. The new copy starts open and is shut at once,
// so it fades like one closed by a click elsewhere.
const openMenus = (within) => Array.from(within.querySelectorAll(".dl-menu.open"), (m) => m.id);
function fadeShut(ids) {
  for (const id of ids) {
    const menu = $(id);
    if (!menu) continue;
    // Opened without its fade: by now the page has been laid out with it
    // shut, so opening it normally would start fading in from nothing, and
    // shutting it again would have nothing to fade from.
    menu.style.transition = "none";
    menu.classList.add("open");
    menu.offsetWidth; // laid out open, so the fade has somewhere to start
    menu.style.transition = "";
    menu.classList.remove("open");
  }
}

// The size picked from the Download button's menu; null is all of them. The
// menu only sets it: the button does the queueing.
let batchPick = null;

function renderControls(now) {
  const ch = activeChannel();
  if (!ch) { controlsEl.innerHTML = ""; return; }

  let tabs = Object.entries(TABS).map(([t, label]) =>
    `<button class="tab ${activeTab === t ? "active" : ""}" data-tab="${t}">${label}</button>`).join("");

  // Each number is what switching that option on would show, so every lit one
  // matches the grid. Members is there even at 0, so the row keeps one shape
  // from channel to channel. The line sets the two pairs apart. Each side has
  // its own fill: All, Public and Members share one, the archive pair the
  // other.
  const lit = {
    all: filters.access === "all" && filters.have === "all",
    public: filters.access === "public",
    members: filters.access === "members",
    archived: filters.have === "archived",
    not: filters.have === "not",
  };
  const pill = (key, label, slot, counts) =>
    `<button class="pill ${lit[key] ? "active" : ""}" data-pick="${key}" data-slot="${slot}">${label} <span class="n">${counts[key]}</span></button>`;
  const group = (counts) => `<div class="filters">${pill("all", "All", "access", counts)}${pill("public", "Public", "access", counts)}${pill("members", "Members", "access", counts)}<span class="pill-sep"></span>${pill("archived", "Archived", "have", counts)}${pill("not", "Not archived", "have", counts)}</div>`;

  // The filters and sort sit on one row; the download button rides the tab
  // strip, where there is room, so the row doesn't wrap at the default width.
  let bar = "";
  if (now) {
    const { jobs, locked } = downloadable(now.shown, archived, { cookies: !!state.cookies_browser, holds: queue.holds });
    // The arrow offers the first few in the order on screen, so the sort
    // picks the batch: Oldest + First 50 is the 50 oldest not yet archived.
    const sizes = batchSizes(jobs.length);
    const take = batchOf(batchPick, jobs.length);
    const menu = sizes.length ? `<button class="btn btn-primary split-arrow" id="dlmore" title="Download only the first few"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"><path d="M6 9l6 6 6-6"/></svg></button>
        <div class="dl-menu" id="dlmenu">
          ${sizes.map((n) => `<button class="cookie-item${take === n ? " active" : ""}" data-first="${n}">First ${n}</button>`).join("")}
          <button class="cookie-item${take === jobs.length ? " active" : ""}" data-first="">All ${jobs.length}</button>
        </div>` : "";
    // "All" of what the filters leave, which the counts beside them already say.
    const label = take < jobs.length ? `first ${take}` : jobs.length === 1 ? "1" : `all ${jobs.length}`;
    // Members-only videos it can't get are left out without a word on the
    // button; pointing at it says how many, and what would bring them in.
    const skipped = locked ? ` title="${locked} members-only ${locked === 1 ? "video isn't" : "videos aren't"} included: ${locked === 1 ? "it needs" : "they need"} cookies. Pick the browser you're signed into YouTube with from the cookie menu in the top bar."` : "";
    const download = jobs.length ? `<div class="split"><button class="btn btn-primary" id="dlshown"${skipped}>↓ Download ${label}</button>${menu}</div>`
      : locked ? `<button class="btn btn-primary" id="dlshown" title="These are members-only. Pick the browser you're signed into YouTube with.">Set cookies to download</button>`
      : ""; // nothing to queue: no button, rather than one that can't be pressed
    // With a search, matches the filters hide can be had back in one click,
    // which switches the filters off. There is nothing more to show once they
    // are, so then the button goes.
    const findAll = query && !lit.all && now.matches > now.shown.length;
    // How many of the matches are on screen, as a browser's find bar says it.
    findN.textContent = !query ? "" : findAll ? `${now.shown.length} of ${now.matches}` : `${now.matches}`;
    bar = `
      ${group(now.counts)}
      <div class="sortsel">
        <button class="sort-btn" id="sort" title="Sort order"><span class="sort-lbls">${Object.entries(SORTS).map(([value, label]) =>
          `<span${filters.sort === value ? "" : ` class="off"`}>${label}</span>`).join("")}</span>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><path d="M6 9l6 6 6-6"/></svg>
        </button>
        <div class="dl-menu" id="sort-menu">${Object.entries(SORTS).map(([value, label]) =>
          `<button class="cookie-item${filters.sort === value ? " active" : ""}" data-sort="${value}">${label}</button>`).join("")}</div>
      </div>
      <span class="tabs-spacer"></span>
      ${findAll ? `<button class="btn btn-low${findAllWas ? "" : " arriving"}" id="find-all" title="Switch the filters off">Show all ${now.matches}</button>` : ""}`;
    findAllWas = findAll;
    tabs += `<span class="tabs-spacer"></span>${download}`;
  } else {
    bar = `<span class="tabs-spacer"></span>`;
    findAllWas = false;
    findN.textContent = "";
  }

  // Where each fill and the tab underline sat, so they can slide from there
  // to the new choice, and which menus were open, so they can fade.
  const was = {};
  controlsEl.querySelectorAll(".pill-on:not(.leaving)").forEach((m) => { was[m.dataset.slot] = whereIs(m); });
  const view = `${ch.url}|${activeTab}`;
  const numbersWere = controlsEl.dataset.view === view
    ? Object.fromEntries(Array.from(controlsEl.querySelectorAll("[data-pick]"), (b) => [b.dataset.pick, Number(b.querySelector(".n").textContent)]))
    : null;
  controlsEl.dataset.view = view;
  const wasTab = whereIs(controlsEl.querySelector(".tab-on"));
  const wasOpen = openMenus(controlsEl);

  // The two rows are made once; their contents are redrawn, and the search
  // box stays where it is at the end of the second.
  if (!controlsEl.querySelector(".row-bar")) {
    controlsEl.innerHTML = `<div class="tabs"></div><div class="controls"><div class="row-bar"></div></div>`;
    controlsEl.querySelector(".controls").append(findEl);
  }
  controlsEl.querySelector(".tabs").innerHTML = tabs;
  controlsEl.querySelector(".row-bar").innerHTML = bar;

  controlsEl.querySelectorAll(".filters").forEach((row) => {
    for (const slot of ["access", "have"]) slideMark(row, "pill-on", row.querySelector(`.pill.active[data-slot="${slot}"]`), was[slot], slot);
  });
  rollNumbers(controlsEl.querySelector(".filters"), numbersWere);
  const tabsRow = controlsEl.querySelector(".tabs");
  slideMark(tabsRow, "tab-on", tabsRow.querySelector(".tab.active"), wasTab);
  fadeShut(wasOpen);

  controlsEl.querySelectorAll("[data-tab]").forEach((b) =>
    b.addEventListener("click", () => switchTab(b.dataset.tab)));
  controlsEl.querySelectorAll("[data-pick]").forEach((b) =>
    b.addEventListener("click", () => pickFilter(b.dataset.pick)));
  // A menu of our own, not the system's: that one opens over the button with
  // the chosen row on top of it, where every other menu here drops below.
  $("sort")?.addEventListener("click", (e) => {
    e.stopPropagation();
    $("sort-menu").classList.toggle("open");
  });
  controlsEl.querySelectorAll("[data-sort]").forEach((b) =>
    b.addEventListener("click", () => setFilter("sort", b.dataset.sort)));
  $("dlshown")?.addEventListener("click", () => downloadShown(batchPick ?? Infinity));
  $("find-all")?.addEventListener("click", () => {
    clearFilters();
    renderList();
  });
  $("dlmore")?.addEventListener("click", (e) => {
    e.stopPropagation();
    $("dlmenu").classList.toggle("open");
  });
  controlsEl.querySelectorAll("[data-first]").forEach((b) =>
    b.addEventListener("click", () => { batchPick = Number(b.dataset.first) || null; renderControls(currentView()); }));
}
document.addEventListener("click", () => {
  $("dlmenu")?.classList.remove("open");
  $("sub-menu")?.classList.remove("open");
  $("sort-menu")?.classList.remove("open");
});

function setFilter(name, value) {
  filters[name] = value;
  localStorage.setItem(`vault-${name}`, value);
  renderList();
}

// Every filter off: what All does, and Show all beside the search.
function clearFilters() {
  for (const name of ["access", "have"]) { filters[name] = "all"; localStorage.setItem(`vault-${name}`, "all"); }
}

// One row of options in two pairs: Public and Members take turns, as do
// Archived and Not archived, since both of a pair on would be everything. One
// of each pair can be on together. All switches every one off, and is lit
// while none is on.
function pickFilter(key) {
  if (key === "all") { clearFilters(); renderList(); return; }
  const name = key === "public" || key === "members" ? "access" : "have";
  setFilter(name, filters[name] === key ? "all" : key);
}

async function switchTab(tab) {
  const ch = activeChannel();
  if (!ch || tab === activeTab) return;
  activeTab = tab;
  localStorage.setItem("vault-tab", tab);
  if (isLazyTab(tab) && !tabVideos(ch, tab)) {
    renderList(); // shows the tab as selected, and "Loading…" under it
    await loadTab(ch, tab);
  }
  renderList();
}

// Fetch the Shorts or Live list for a channel. A failure is recorded as a
// failure: caching it as an empty list is what made a bot check or a dropped
// connection read as "No shorts on this channel".
async function loadTab(ch, tab) {
  const key = tabId(ch, tab);
  if (tabLoading.has(key)) return;
  tabLoading.add(key);
  tabErrors.delete(key);
  try {
    const info = await invoke("fetch_channel", { url: ch.url, tab, cookies: state.cookies_browser ?? null });
    const entry = tabCache.get(ch.url) || {};
    entry[tabKey(tab)] = info.videos || [];
    tabCache.set(ch.url, entry);
  } catch (e) {
    tabErrors.set(key, String(e));
  } finally {
    tabLoading.delete(key);
  }
}

// The videos behind the cards on screen, by id. Clicks resolve through this
// rather than through one tab's list: the handler used to look only in the
// Videos tab, so every button on Shorts and Live did nothing.
let gridVideos = new Map();

function renderGrid(now) {
  const ch = activeChannel();
  if (!ch) return;
  const message = (title, body) => {
    gridEl.style.display = "none";
    emptyEl.style.display = "flex";
    emptyEl.innerHTML = `${MARK}<h2>${title}</h2>${body}`;
  };

  const failed = tabErrors.get(tabId(ch, activeTab));
  if (failed) {
    message(`Couldn't load ${TABS[activeTab]}`, `<p>${esc(failed)}</p>
      <div style="margin-top:18px"><button class="btn btn-primary" id="tab-retry">Try again</button></div>`);
    $("tab-retry").addEventListener("click", async () => {
      await loadTab(ch, activeTab);
      renderList();
    });
    return;
  }
  if (!now) {
    message("One moment", `<p>Loading ${TABS[activeTab]}…</p><div class="spin"></div>`);
    return;
  }
  const wanted = pasted && !pasted.listed && pasted.channel === ch.url && query === pasted.title ? pasted : null;
  if (!now.shown.length && wanted) {
    const unlisted = wanted.availability === "unlisted";
    message(unlisted ? "An unlisted video" : "Not on the channel’s pages", `<p>“${esc(wanted.title)}” ${unlisted
      ? "is unlisted: only its link finds it, so it isn't on the channel's tabs"
      : "isn't listed on any of this channel's tabs"}. It can still be downloaded into this channel's folder.</p>
      <div style="margin-top:18px"><button class="btn btn-primary" id="dl-pasted">↓ Download it</button></div>`);
    $("dl-pasted").addEventListener("click", () => startDownload({ id: wanted.id, title: wanted.title, url: wanted.url }));
    return;
  }
  if (!now.shown.length) {
    const unfiltered = filters.access === "all" && filters.have === "all";
    message("Nothing here", `<p>${query
      ? `No ${TABS[activeTab].toLowerCase()} match “${esc(query.trim())}”${unfiltered ? "" : " with these filters"}.`
      : unfiltered
      ? `No ${TABS[activeTab].toLowerCase()} on this channel.`
      : "No videos match these filters."}</p>`);
    return;
  }

  emptyEl.style.display = "none";
  gridEl.style.display = "grid";
  gridVideos = new Map(now.shown.map((v) => [v.id, v]));
  gridList = now.shown;
  gridCells = new Map(); // a card's contents may have changed, so none is reused
  gridDrawn = null;
  drawWindow();
}

// A channel can list thousands of videos, and drawing a card for each made
// every filter and sort click wait on all of them. Only the rows near the
// view are on the page — see windowOf — and they are swapped as it scrolls.
let gridList = [];         // what the filters show, in order
let gridCells = new Map(); // id -> the card on the page
let gridDrawn = null;      // "first-last" of the cards on the page
let gridShape = null;      // { cols, rowH }, measured from a drawn card
const mainEl = document.querySelector(".main");

function cellFor(v) {
  let el = gridCells.get(v.id);
  if (el) return el;
  const t = document.createElement("template");
  t.innerHTML = card(v).trim();
  el = t.content.firstElementChild;
  gridCells.set(v.id, el);
  return el;
}

function drawWindow() {
  if (gridEl.style.display === "none" || !gridList.length) return;
  if (!gridShape) {
    // Every card is the same height, so one is enough to measure by.
    if (!gridEl.firstElementChild) gridEl.replaceChildren(cellFor(gridList[0]));
    const css = getComputedStyle(gridEl);
    gridShape = {
      cols: css.gridTemplateColumns.split(" ").length,
      // To the fraction of a pixel: a rounded height is off by a little per
      // row, and the cards would shift by that each time the window moved.
      rowH: gridEl.firstElementChild.getBoundingClientRect().height + parseFloat(css.rowGap),
    };
    gridDrawn = null;
  }
  const viewH = mainEl.clientHeight;
  // Where the cards begin, not the box: the box's top padding stands in for
  // the rows above the window.
  const top = mainEl.getBoundingClientRect().top - gridEl.getBoundingClientRect().top;
  const w = windowOf({ count: gridList.length, ...gridShape, top, viewH, over: viewH });
  const key = `${w.first}-${w.last}`;
  if (key === gridDrawn) return;
  gridDrawn = key;
  const cells = new Map();
  for (const v of gridList.slice(w.first, w.last)) cells.set(v.id, cellFor(v));
  gridCells = cells;
  gridEl.style.paddingTop = `${w.before}px`;
  gridEl.style.paddingBottom = `${w.after}px`;
  gridEl.replaceChildren(...cells.values());
}

let gridFrame = null;
const drawWindowSoon = () => { gridFrame ??= requestAnimationFrame(() => { gridFrame = null; drawWindow(); }); };
mainEl.addEventListener("scroll", drawWindowSoon, { passive: true });
// A new width changes how many cards fit across and how tall each is.
let gridWidth = 0;
new ResizeObserver(() => {
  if (gridEl.clientWidth !== gridWidth) { gridWidth = gridEl.clientWidth; gridShape = null; }
  drawWindowSoon();
}).observe(mainEl);

function card(v) {
  const entry = archived.get(v.id);
  const have = archiveState(v.id, archived);
  const playable = have === "have" && !!entry?.path;
  // The exact upload date once the video is archived, the listing's
  // approximate one until then; pointing at it says which, as a date.
  const when = entry?.timestamp ?? v.timestamp;
  const day = when && new Date(when * 1000).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  const exactly = entry?.timestamp ? `Uploaded ${day}`
    : entry ? `Uploaded around ${day}. Sync fills in the exact day.`
    : `Uploaded around ${day}. The exact day is known once it's downloaded.`;
  const date = ago(when) && `<span title="${exactly}">${ago(when)}</span>`;
  const meta = [
    fmtViews(v.view_count) && `${fmtViews(v.view_count)} views`,
    date,
    v.unlisted && v.availability !== "unlisted" && "Missing on YT",
  ].filter(Boolean).join(" · ");
  return `
    <div class="card" data-id="${esc(v.id)}">
      <div class="thumb ${playable ? "playable" : ""}" ${playable ? `data-act="play" title="Play ${esc(v.title)}"` : ""}>
        <img loading="lazy" src="${thumb(v.id)}" alt="">
        ${fmtDur(v.duration) ? `<span class="dur">${fmtDur(v.duration)}</span>` : ""}
        ${isMembers(v) ? `<span class="badge">Members only</span>` : ""}
        ${v.availability === "unlisted" ? `<span class="badge unlisted">Unlisted</span>` : ""}
        ${have === "missing" ? `<span class="badge gone">File missing</span>` : ""}
        ${playable ? '<span class="play">▶</span>' : ""}
      </div>
      <div class="card-body">
        <h3 title="${esc(v.title)}">${esc(v.title)}</h3>
        <div class="sub">${meta || "&nbsp;"}</div>
        <div class="card-foot">${cardButton(v)}</div>
      </div>
    </div>`;
}

// The one button under a card, for whatever the video's state is. Drawn here
// for a new card and again by syncCard when a download moves, so the two can't
// disagree about what a state looks like.
function cardButton(v) {
  const id = esc(v.id);
  const job = queue.jobs.get(v.id);
  const have = archiveState(v.id, archived);
  if (job?.status === "downloading") {
    return `<button class="dl busy" data-id="${id}" style="--pct:${Math.round(job.pct)}%">↓ ${Math.round(job.pct)}%</button>`;
  }
  if (job?.status === "queued") return `<button class="dl on" data-id="${id}">• Queued</button>`;
  if (have === "have") {
    // Just finished: archived, but the file's path arrives with the next
    // archive read a moment later.
    return archived.get(v.id).path
      ? `<button class="dl on" data-id="${id}" data-act="play" title="Play here. ⌘-click opens it in a tab without leaving the channel.">▶ Play</button>`
      : `<button class="dl on" data-id="${id}">✓ Archived</button>`;
  }
  // Members-only downloads need cookies; with a browser set they're a normal
  // download, without one they're a guaranteed failure so they stay disabled.
  if (isMembers(v) && !state.cookies_browser) {
    return `<button class="dl" data-id="${id}" disabled title="This video is for channel members only — pick a browser in the cookie menu (top bar) to download it with your membership.">Members only</button>`;
  }
  if (job?.status === "error") return `<button class="dl" data-id="${id}" data-act="dl" title="${esc(job.stat)}">↻ Retry</button>`;
  if (have === "missing") {
    return `<button class="dl" data-id="${id}" data-act="dl" title="The archive records this video but its file isn't in the folder.">↻ Download again</button>`;
  }
  return `<button class="dl" data-id="${id}" data-act="dl">↓ Download</button>`;
}

function syncCard(id) {
  const b = gridEl.querySelector(`.dl[data-id="${CSS.escape(id)}"]`);
  const v = gridVideos.get(id);
  if (b && v) b.outerHTML = cardButton(v);
}

gridEl.addEventListener("click", (e) => {
  const target = e.target.closest("[data-act]");
  if (!target) return;
  const id = target.closest(".card").dataset.id;
  if (target.dataset.act === "play") openVideo(id, { background: e.metaKey });
  else if (gridVideos.has(id)) startDownload(gridVideos.get(id));
});

/* ---------------- playback ---------------- */
// Videos play inside Vault, each in a tab that keeps its place. mpv draws the
// picture in a view under the page (src-tauri/src/player.rs) and the page
// leaves a hole for it; the tabs and controls here are ordinary HTML.
//
// An in-page <video> can't do this job: the web view has no AV1 decoder on a
// Mac without hardware AV1, which is most of what YouTube serves.
//
// The open tabs and their places are this window's own state, like the theme,
// so they're kept in local storage and not in a channel's videos.json.
const TABS_KEY = "vault-open";
const savedTabs = () => { try { return JSON.parse(localStorage.getItem(TABS_KEY)); } catch { return null; } };
const tabs = createTabs(savedTabs());
const saveTabs = () => localStorage.setItem(TABS_KEY, JSON.stringify(tabs.snapshot()));

const playerEl = $("player");
const stageEl = $("stage");
const stageMsg = $("stage-msg");
const seekEl = $("pseek");
const PLAY_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.5v15l13-7.5z"/></svg>`;
const PAUSE_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M6 4.5h4.5v15H6zM13.5 4.5H18v15h-4.5z"/></svg>`;
const REPLAY_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M3 12a9 9 0 1 0 2.6-6.4"/><path d="M3 4v5h5"/></svg>`;

const EXPAND_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/></svg>`;
const SHRINK_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5"/></svg>`;

const watching = () => !playerEl.hidden;
let full = false;          // the picture has the whole screen, bars over it
let windowWasFull = false; // the window was already fullscreen, so leaving keeps it
let idle = null;
let loadedId = null;   // the tab whose file mpv holds
let reported = false;  // the loaded file has reported in at least once
let loads = 0;         // counts loads, so one overtaken by a newer one stops
let muted = false;
const coverEl = $("cover");
const VOLUME_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 9v6h4l5 4V5L8 9z"/><path d="M16.5 8.5a5 5 0 0 1 0 7"/></svg>`;
const MUTED_ICON = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 9v6h4l5 4V5L8 9z"/><path d="M16 9.5l5 5M21 9.5l-5 5"/></svg>`;
// A slider's track is filled up to its handle.
const fill = (el) => el.style.setProperty("--fill", `${((el.value - el.min) / ((el.max - el.min) || 1)) * 100}%`);
let playing = { paused: true, ended: false };
let seeking = false;   // the seek bar is being dragged, so reports don't move it
let poll = null;
let lastSave = 0;

function renderWatching() {
  const n = tabs.tabs.length;
  $("watching").hidden = n === 0;
  $("watching").innerHTML = `Open videos <span class="n">${n}</span>`;
}

function renderTabs() {
  $("ptabs").innerHTML = tabs.tabs.map((t) => `
    <div class="ptab ${t.id === tabs.activeId ? "active" : ""}" data-id="${esc(t.id)}">
      <span class="t">${esc(t.title)}</span>
      <button class="x" data-close title="Close this tab">✕</button>
    </div>`).join("");
  renderWatching();
}

function renderTransport() {
  const tab = tabs.active;
  $("pfull").innerHTML = full ? SHRINK_ICON : EXPAND_ICON;
  $("pfull").title = full ? "Leave fullscreen (F or Esc)" : "Fullscreen (F)";
  $("pmute").innerHTML = muted ? MUTED_ICON : VOLUME_ICON;
  $("pmute").title = muted ? "Unmute (M)" : "Mute (M)";
  const btn = $("pplay");
  btn.innerHTML = playing.ended ? REPLAY_ICON : playing.paused ? PLAY_ICON : PAUSE_ICON;
  btn.title = playing.ended ? "Play again (Space)" : playing.paused ? "Play (Space)" : "Pause (Space)";
  $("ptime").textContent = `${fmtDur(tab?.position ?? 0)} / ${fmtDur(tab?.duration ?? 0)}`;
  if (!seeking) {
    seekEl.max = Math.max(1, tab?.duration ?? 0);
    seekEl.value = tab?.position ?? 0;
    fill(seekEl);
  }
}

// Tells the backend where the hole is. Called whenever the stage moves.
function placeVideo() {
  if (!watching()) return;
  const r = stageEl.getBoundingClientRect();
  invoke("player_place", { visible: true, x: r.left, top: r.top, w: r.width, h: r.height, winH: window.innerHeight })
    .catch((e) => console.error("Could not place the video:", e));
}
new ResizeObserver(placeVideo).observe(stageEl);

// The mark that blinks in the middle of the picture when playback starts or
// stops, or the sound goes off or on, as on YouTube.
function flash(icon) {
  const el = $("bezel");
  el.innerHTML = icon;
  el.classList.remove("go");
  void el.offsetWidth; // restarts the animation when toggled twice quickly
  el.classList.add("go");
}

function stageError(err) {
  // Drawn over the hole, so it has its own ground.
  stageMsg.hidden = false;
  stageMsg.innerHTML = `<h3>This video couldn't be played</h3><p>${esc(err)}</p>`;
}

// Puts the active tab's video on, from where it was left. `paused` holds it
// on that frame, for a tab that came forward without being asked to play.
async function loadActive({ paused = false } = {}) {
  const tab = tabs.active;
  if (!tab) return;
  const mine = ++loads;
  stageMsg.hidden = true;
  try {
    if (loadedId === tab.id) await invoke("player_set", { action: paused ? "pause" : "play" });
    else {
      // The stage is covered until the new file reports in (tick lifts it),
      // with the frame this tab was left on, or its thumbnail before it has
      // one. The cover goes up only once its picture is ready, so the switch
      // goes from picture to picture and never through black.
      const left = tab.still && resumeAt(tab) === tab.position;
      await showCover(left ? convertFileSrc(tab.still) : thumb(tab.id), thumb(tab.id));
      if (mine !== loads) return;
      stageEl.classList.add("loading");
      reported = false;
      await invoke("player_load", { path: tab.path, start: resumeAt(tab), paused });
      if (mine !== loads) return;
      loadedId = tab.id;
    }
    playing = { paused, ended: false };
  } catch (e) {
    if (mine !== loads) return;
    loadedId = null;
    playing = { paused: true, ended: false };
    stageError(e);
  }
  renderTransport();
}

// Readies the cover's picture. A still that's gone (the system clears its
// temporary files) falls back to the thumbnail, and that to plain black.
async function showCover(src, fallback) {
  for (const url of new Set([src, fallback])) {
    coverEl.src = url;
    // A thumbnail comes over the network; a slow one mustn't hold the video.
    const slow = new Promise((_, no) => setTimeout(no, 400));
    try { await Promise.race([coverEl.decode(), slow]); return; } catch { /* try the next */ }
  }
  coverEl.removeAttribute("src");
}

// Saves the frame on screen as the active tab's still, for its next cover.
async function keepStill() {
  const tab = tabs.active;
  if (!tab || loadedId !== tab.id || !reported) return;
  try { tabs.keep(tab.id, await invoke("player_still", { id: tab.id })); }
  catch (e) { console.warn("No still saved:", e); }
}

async function tick() {
  let s;
  try { s = await invoke("player_state"); } catch { return; }
  muted = !!s.muted;
  const tab = tabs.active;
  // A report about another file crossed a tab switch, and one with no length
  // came before the file opened. Either would overwrite this tab's place.
  if (!tab || loadedId !== tab.id || s.path !== tab.path || !(s.duration > 0)) return;
  // Until the frame is really there the cover stays, and the position is
  // not taken: a seek into a long video lands a while after the file opens,
  // and what's reported meanwhile isn't where the tab was left.
  if (!s.ready) return;
  stageEl.classList.remove("loading");
  // Seen here and not where the button is pressed, so a pause that came from
  // the keyboard's media key shows too.
  if (reported && s.paused !== playing.paused && !s.ended) flash(s.paused ? PAUSE_ICON : PLAY_ICON);
  reported = true;
  tabs.note(tab.id, s.position, s.duration);
  playing = { paused: s.paused, ended: s.ended };
  renderTransport();
  if (Date.now() - lastSave > 5000) { saveTabs(); lastSave = Date.now(); }
}

// Arrives paused unless a card's Play asked for the video.
function showPlayer({ paused = true } = {}) {
  if (!tabs.active) return;
  playerEl.hidden = false;
  root.classList.add("watching");
  renderTabs();
  placeVideo();
  invoke("player_set", { action: "volume", value: Number($("pvol").value) }).catch(() => {});
  loadActive({ paused });
  clearInterval(poll);
  poll = setInterval(tick, 250);
}

// In fullscreen the bars lie over the picture and go away, with the pointer,
// once it has been still for a moment. They stay while the video is paused or
// the pointer is on one of them.
function wake() {
  playerEl.classList.remove("idle");
  clearTimeout(idle);
  if (!full) return;
  idle = setTimeout(() => {
    if (playing.paused || playerEl.querySelector(".ptop:hover, .pbar:hover")) return wake();
    playerEl.classList.add("idle");
    hideTip();
  }, 2500);
}

// Fullscreen is the window's own fullscreen plus the bars moving onto the
// picture. `window: false` is for when the window already left by itself.
async function setFull(on, { window: switchWindow = true } = {}) {
  if (on === full || (on && !watching())) return;
  if (on) windowWasFull = await invoke("player_fullscreen", { on: null }).catch(() => false);
  full = on;
  playerEl.classList.toggle("full", on);
  if (switchWindow && !windowWasFull) {
    invoke("player_fullscreen", { on }).catch((e) => console.error(e));
  }
  wake();
  renderTransport();
}

// The window can leave fullscreen without being asked (the green button, the
// system's own shortcut). Checked once a resize has settled, since the answer
// is in flux while the window animates.
let resized = null;
window.addEventListener("resize", () => {
  clearTimeout(resized);
  resized = setTimeout(async () => {
    if (!full || windowWasFull) return;
    const still = await invoke("player_fullscreen", { on: null }).catch(() => true);
    if (!still) setFull(false, { window: false });
  }, 1000);
});

// Back to the channel. The video pauses: it would otherwise play on unseen.
async function leavePlayer() {
  if (!watching()) return;
  await setFull(false);
  clearInterval(poll);
  await tick();
  await keepStill();
  saveTabs();
  playerEl.hidden = true;
  root.classList.remove("watching");
  invoke("player_set", { action: "pause" }).catch(() => {});
  invoke("player_place", { visible: false, x: 0, top: 0, w: 0, h: 0, winH: 0 }).catch(() => {});
  renderWatching();
}

// Switching tabs brings the video up paused where it was left. Only Play on a
// card starts one playing unasked.
async function selectTab(id) {
  if (id === tabs.activeId) return;
  await tick();
  await keepStill();
  tabs.select(id);
  saveTabs();
  renderTabs();
  loadActive({ paused: true });
}

async function closeTab(id) {
  const wasActive = id === tabs.activeId;
  tabs.close(id);
  saveTabs();
  if (wasActive) {
    loadedId = null;
    await invoke("player_set", { action: "stop" }).catch(() => {});
  }
  if (!tabs.active) { await leavePlayer(); renderWatching(); return; }
  renderTabs();
  if (wasActive && watching()) loadActive({ paused: true });
}

// Play on a card. With ⌘ held the video opens in a tab without leaving the
// channel, the way a browser opens a link in the background.
async function openVideo(id, { background = false } = {}) {
  const entry = archived.get(id);
  if (!entry?.path) return;
  if (watching() && !background) { await tick(); await keepStill(); }
  tabs.open({ id, path: entry.path, title: gridVideos.get(id)?.title ?? entry.title ?? id }, { background });
  saveTabs();
  if (background) { renderWatching(); return; }
  if (watching()) { renderTabs(); loadActive(); } else showPlayer({ paused: false });
}

async function togglePlay() {
  const tab = tabs.active;
  if (!tab) return;
  if (loadedId !== tab.id) return loadActive();
  try {
    if (playing.ended) await invoke("player_set", { action: "seek", value: 0 });
    await invoke("player_set", { action: playing.paused || playing.ended ? "play" : "pause" });
  } catch (e) { stageError(e); }
  await tick();
  wake();
}

async function toggleMute() {
  try {
    await invoke("player_set", { action: "mute" });
    muted = !!(await invoke("player_state")).muted;
    flash(muted ? MUTED_ICON : VOLUME_ICON);
  } catch (e) { console.error("Could not mute:", e); }
  renderTransport();
}

function seekTo(seconds) {
  const tab = tabs.active;
  if (!tab || loadedId !== tab.id) return;
  const to = Math.min(Math.max(0, seconds), Math.max(0, tab.duration - 0.5));
  tabs.note(tab.id, to, tab.duration);
  invoke("player_set", { action: "seek", value: to }).catch((e) => stageError(e));
}

$("watching").addEventListener("click", () => showPlayer());
$("pback").addEventListener("click", leavePlayer);
$("pplay").addEventListener("click", togglePlay);
stageEl.addEventListener("click", (e) => { if (e.target === stageEl) togglePlay(); });
$("pfull").addEventListener("click", () => setFull(!full));
playerEl.addEventListener("mousemove", wake);
$("ptabs").addEventListener("click", (e) => {
  const tab = e.target.closest(".ptab");
  if (!tab) return;
  if (e.target.closest("[data-close]")) closeTab(tab.dataset.id);
  else selectTab(tab.dataset.id);
});
// Resting on a tab shows its whole title, which the tab itself cuts short, and
// how far through it is.
const tipEl = $("ptip");
let tipTimer = null;
let tipSeen = 0; // when a card was last showing
function hideTip() {
  clearTimeout(tipTimer);
  if (!tipEl.hidden) tipSeen = Date.now();
  tipEl.hidden = true;
}
$("ptabs").addEventListener("mouseover", (e) => {
  const el = e.target.closest(".ptab");
  const tab = el && tabs.find(el.dataset.id);
  // The gap between two tabs: keep the card that's up.
  if (!tab) return;
  clearTimeout(tipTimer);
  // The first card waits for the pointer to rest. Once one is up, or was a
  // moment ago, the next tab's comes at once, so tabs can be flipped through.
  const warm = !tipEl.hidden || Date.now() - tipSeen < 600;
  tipTimer = setTimeout(() => {
    const r = el.getBoundingClientRect();
    tipEl.innerHTML = `<b>${esc(tab.title)}</b><img src="${thumb(tab.id)}" alt=""><i style="width:${(progress(tab) * 100).toFixed(1)}%"></i>`;
    // A delisted video has no thumbnail left on YouTube.
    tipEl.querySelector("img").addEventListener("error", (ev) => ev.target.remove());
    tipEl.hidden = false;
    // Hangs from the bottom edge of the bar the tabs sit in, level with the
    // top of the picture.
    tipEl.style.top = `${$("ptabs").parentElement.getBoundingClientRect().bottom}px`;
    tipEl.style.left = `${Math.max(8, Math.min(r.left, window.innerWidth - tipEl.offsetWidth - 8))}px`;
  }, warm ? 0 : 400);
});
$("ptabs").addEventListener("mouseleave", hideTip);
$("ptabs").addEventListener("mousedown", hideTip);

// Pointing along the seek bar shows the time a click there would land on,
// and fills the bar up to the pointer.
const HANDLE = 13; // the handle's width: its centre can't reach the bar's ends
const seekTip = $("pseek-tip");
seekEl.addEventListener("pointermove", (e) => {
  const duration = tabs.active?.duration ?? 0;
  if (!(duration > 0)) { seekEl.style.setProperty("--ghost", "0px"); return; }
  const r = seekEl.getBoundingClientRect();
  const along = Math.min(1, Math.max(0, (e.clientX - r.left - HANDLE / 2) / (r.width - HANDLE)));
  seekTip.textContent = fmtDur(along * duration);
  seekTip.hidden = false;
  const x = r.left + HANDLE / 2 + along * (r.width - HANDLE);
  seekEl.style.setProperty("--ghost", `${x - r.left}px`);
  seekTip.style.left = `${Math.max(6, Math.min(x - seekTip.offsetWidth / 2, window.innerWidth - seekTip.offsetWidth - 6))}px`;
  seekTip.style.top = `${r.top - seekTip.offsetHeight - 6}px`;
});
seekEl.addEventListener("pointerleave", () => { seekTip.hidden = true; });

seekEl.addEventListener("pointerdown", () => { seeking = true; });
seekEl.addEventListener("input", () => { fill(seekEl); seekTo(Number(seekEl.value)); $("ptime").textContent = `${fmtDur(Number(seekEl.value))} / ${fmtDur(tabs.active?.duration ?? 0)}`; });
seekEl.addEventListener("change", () => { seeking = false; });
$("pvol").value = localStorage.getItem("vault-volume") ?? 100;
fill($("pvol"));
$("pmute").addEventListener("click", toggleMute);
$("pvol").addEventListener("input", (e) => {
  fill(e.target);
  localStorage.setItem("vault-volume", e.target.value);
  invoke("player_set", { action: "volume", value: Number(e.target.value) }).catch(() => {});
});
document.addEventListener("keydown", (e) => {
  if (!watching() || e.metaKey || e.ctrlKey || e.altKey) return;
  // YouTube's keys: arrows skip 5 seconds, J and L skip 10, K is play/pause.
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const step = { ArrowLeft: -5, ArrowRight: 5, j: -10, l: 10 }[key];
  if (key === " " || key === "k") togglePlay();
  else if (key === "f") setFull(!full);
  else if (key === "m") toggleMute();
  // Esc steps out one level: out of fullscreen first, then back to the channel.
  else if (key === "Escape") full ? setFull(false) : leavePlayer();
  else if (step) seekTo((tabs.active?.position ?? 0) + step);
  else return;
  e.preventDefault();
  wake();
  // A focused slider or button would act on the same key a second time.
  document.activeElement?.blur?.();
});
window.addEventListener("beforeunload", saveTabs);

/* ---------------- download folder ---------------- */
// Pointing a channel at a folder also stamps that folder with the channel's
// identity and creates its archive, so the directory is self-describing.
async function adoptDir(ch, dir) {
  ch.download_dir = dir;
  await invoke("set_channel_dir", { url: ch.url, title: ch.title, dir })
    .catch((e) => console.error("Could not stamp the channel folder:", e));
  await persist();
  // A different folder is a different archive.
  await loadArchive();
  if (activeChannel() === ch) { renderHeader(ch); renderList(); }
}

async function ensureDir(ch) {
  if (ch.download_dir) return ch.download_dir;
  let suggested = "";
  try {
    const base = await invoke("default_download_dir_str");
    suggested = `${base}/${safeName(ch.title)}`;
    await invoke("ensure_dir", { dir: suggested });
  } catch { suggested = undefined; }
  const picked = await open({
    directory: true, multiple: false, defaultPath: suggested,
    title: `Where should ${ch.title} downloads go?`,
  });
  if (!picked) return null;
  await adoptDir(ch, picked);
  return picked;
}

async function changeDir(ch) {
  let defaultPath = ch.download_dir;
  if (!defaultPath) {
    try { defaultPath = await invoke("default_download_dir_str"); } catch { defaultPath = undefined; }
  }
  const picked = await open({ directory: true, multiple: false, defaultPath, title: "Choose the download folder" });
  if (!picked) return;
  await adoptDir(ch, picked);
}

/* ---------------- queue engine (one at a time) ---------------- */
// Where a video's file goes. One the folder records but has lost goes back to
// the folder that recorded it, which may not be the channel's current one.
function jobFor(v, dir) {
  const missing = archiveState(v.id, archived) === "missing";
  return { id: v.id, title: v.title, url: v.url, dir: (missing && archived.get(v.id).dir) || dir };
}

async function startDownload(v) {
  const ch = activeChannel();
  if (!ch || queue.holds(v.id)) return;
  const dir = await ensureDir(ch);
  if (!dir) return;
  if (!queue.add(jobFor(v, dir))) return;
  renderQueue();
  syncCard(v.id);
  renderControls(currentView()); // one fewer for "Download all N"
  pump();
}

// Queues what the filters have narrowed to. With nothing but locked
// members-only videos on screen, the button asks for cookies instead.
// Queues what's on screen and not on disk; `first` keeps only that many, in
// the order shown.
async function downloadShown(first = Infinity) {
  const ch = activeChannel();
  const now = currentView();
  if (!ch || !now) return;
  const { jobs, locked } = downloadable(now.shown, archived, { cookies: !!state.cookies_browser, holds: queue.holds });
  if (!jobs.length) {
    if (locked) setTimeout(() => cookieMenu.classList.add("open")); // after this click's own close-all
    return;
  }
  const batch = jobs.slice(0, first);
  if (batch.length > ASK_ABOVE) {
    const go = await ask(
      `This adds ${batch.length} videos to the queue. They download one at a time, so it can run for hours and take a lot of disk space. The arrow next to the button picks a smaller batch.`,
      { title: `Queue ${batch.length} videos?`, kind: "warning", okLabel: QUEUE_THEM, cancelLabel: "Cancel" });
    if (!go) return;
  }
  const dir = await ensureDir(ch);
  if (!dir) return;
  for (const { video } of batch) queue.add(jobFor(video, dir));
  renderQueue();
  renderList();
  pump();
}

const QUEUE_THEM = "Queue them";

function pump() {
  const job = queue.next();
  if (!job) return;
  updateJobElement(job.id);
  renderSummary();
  syncCard(job.id);
  invoke("download_video", { id: job.id, url: job.url, dir: job.dir, cookies: state.cookies_browser ?? null })
    .catch((e) => finish(job.id, false, `Error: ${e}`));
}

function setJob(id, patch) {
  const j = queue.jobs.get(id);
  if (!j) return;
  Object.assign(j, patch);
  updateJobElement(id);
  renderSummary();
  syncCard(id);
}
function finish(id, ok, stat) {
  if (!queue.settle(id, ok, stat)) return;
  // Marks the card at once, but only when the file landed in the channel on
  // screen — the archive is per channel. The path isn't known here; the
  // archive read below supplies it and turns the card into Play.
  if (ok) {
    const ch = activeChannel();
    const j = queue.jobs.get(id);
    if (ch && j.dir && ch.download_dir === j.dir && !archived.get(id)?.path) {
      archived.set(id, { id, path: null, pending: true, dir: j.dir });
    }
  }
  updateJobElement(id);
  renderSummary();
  syncCard(id);
  pump();
  if (ok) refreshArchiveSoon();
  else if (activeChannel()) renderControls(currentView()); // a failed job can be queued again
}

// Coalesces the burst of archive reads that a big batch would otherwise
// trigger — one reload shortly after the last completion instead of one per
// video.
let archiveRefresh = null;
function refreshArchiveSoon() {
  if (archiveRefresh) clearTimeout(archiveRefresh);
  archiveRefresh = setTimeout(async () => {
    archiveRefresh = null;
    await loadArchive();
    if (activeChannel()) settleList();
  }, 600);
}

// A redraw for a change nobody clicked, like a download finishing under Not
// archived: the cards that leave fade out, then the rest slide to their new
// places and any that arrive fade in. A click redraws at once, through
// renderList.
let settling = 0;
async function settleList() {
  const turn = ++settling;
  const before = new Map(Array.from(gridEl.querySelectorAll(".card"), (el) => [el.dataset.id, el.getBoundingClientRect()]));
  const now = currentView();
  const staying = new Set(now?.shown.map((v) => v.id));
  const leaving = Array.from(gridEl.querySelectorAll(".card")).filter((el) => !staying.has(el.dataset.id));
  if (leaving.length) {
    await Promise.all(leaving.map((el) => el.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 180, easing: "ease-out", fill: "forwards" }).finished));
    if (turn !== settling) return; // a later redraw took over
  }
  renderList();
  for (const el of gridEl.querySelectorAll(".card")) {
    const was = before.get(el.dataset.id);
    if (!was) { if (before.size) el.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 180, easing: "ease-out" }); continue; }
    const is = el.getBoundingClientRect();
    const dx = was.left - is.left, dy = was.top - is.top;
    if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue;
    el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], { duration: 250, easing: "ease-out" });
  }
}

/* ---------------- queue rendering ---------------- */
const queueEl = $("queue");

const GRIP_ICON = '<svg width="8" height="14" viewBox="0 0 8 14" fill="currentColor"><circle cx="2" cy="2" r="1.2"/><circle cx="6" cy="2" r="1.2"/><circle cx="2" cy="7" r="1.2"/><circle cx="6" cy="7" r="1.2"/><circle cx="2" cy="12" r="1.2"/><circle cx="6" cy="12" r="1.2"/></svg>';

// A waiting row says the line is held, so "Queued" never reads as "about to start".
const statOf = (j) =>
  j.status !== "queued" || j.stat !== "Queued" ? j.stat
  : queue.paused ? "Queued · Paused"
  : queue.resting && j.rested ? `Bot check · again at ${clock(restUntil)}`
  : j.stat;

function renderQueue() {
  if (queue.jobs.size === 0) {
    queueEl.innerHTML = `<div class="qempty">Nothing queued yet.</div>`;
    renderSummary();
    return;
  }
  queueEl.innerHTML = Array.from(queue.jobs.entries()).map(([id, j]) => {
    const controls =
      j.status === "done" ? '<span class="okmark">✓</span>'
      : j.status === "error"
        ? `<button class="qbtn retry" data-retry="${id}">⊘ Retry</button><button class="x" data-dismiss="${id}">✕</button>`
        : `<button class="x" data-cancel="${id}">✕</button>`;
    return `
    <div class="qi" data-id="${id}" data-status="${j.status}">
      ${j.status === "queued" ? `<span class="grip live" data-grip title="Drag to change the order">${GRIP_ICON}</span>` : '<span class="grip"></span>'}
      <img src="${thumb(id)}" alt="">
      <div class="m">
        <div class="t">${esc(j.title)}</div>
        <div class="bar"><i class="${j.status === "done" ? "done" : ""}" style="width:${j.pct}%"></i></div>
        <div class="stat">
          <span class="grow ${j.status === "error" ? "reason" : ""}" title="${esc(statOf(j))}">${esc(statOf(j))}</span>
          ${controls}
        </div>
      </div>
    </div>`;
  }).join("");
  if (drag) markDrop();
  renderSummary();
}

function updateJobElement(id) {
  const j = queue.jobs.get(id);
  const el = queueEl.querySelector(`.qi[data-id="${CSS.escape(id)}"]`);
  // A row's buttons depend on its status (cancel while live, Retry and dismiss
  // once failed, a tick once done), so a change of status redraws the list.
  // Patching only the bar and text left a failed row with a dead ✕ and no
  // Retry until something else happened to redraw it.
  if (!j || !el || el.dataset.status !== j.status) return renderQueue();
  const bar = el.querySelector(".bar > i");
  bar.style.width = j.pct + "%";
  bar.className = j.status === "done" ? "done" : "";
  const g = el.querySelector(".stat .grow");
  g.textContent = statOf(j);
  g.title = statOf(j);
  g.className = "grow" + (j.status === "error" ? " reason" : "");
}

function renderSummary() {
  const list = Array.from(queue.jobs.values());
  const down = list.filter((j) => j.status === "downloading").length;
  const q = list.filter((j) => j.status === "queued").length;
  const done = list.filter((j) => j.status === "done").length;
  const err = list.filter((j) => j.status === "error").length;
  const live = down + q;
  $("qsum").textContent = queue.jobs.size === 0
    ? "Idle"
    : [queue.paused && live ? `Paused · ${live} waiting`
       : queue.resting ? `Bot check · again at ${clock(restUntil)} · ${q} waiting`
       : [`${down} downloading`, `${q} queued`].join(" · "),
       done && `${done} done`, err && `${err} failed`].filter(Boolean).join(" · ");

  // Pause all holds the line; while held, the same button releases it.
  const stop = $("qstop");
  const act = lineAction({ paused: queue.paused, live, failed: err });
  stop.dataset.act = act ?? "";
  stop.disabled = !act;
  const face = act === "resume" ? `${PLAY_ICON} Resume all` : act === "retry" ? `Retry ${err} failed` : `${PAUSE_ICON} Pause all`;
  if (stop.dataset.face !== face) { stop.dataset.face = face; stop.innerHTML = face; }
  // A failed download is unfinished, so it counts as queue, not as finished.
  $("clearq").disabled = !(q || err);
  $("clear").disabled = !done;
  if (unfinished !== live + err) {
    // Something queued when nothing was left to do brings a hidden panel
    // back, so the new download is seen starting. Finished rows don't count.
    if (!unfinished) sideShown = true;
    unfinished = live + err;
    renderSide();
  }
  saveQueueSoon();
}

queueEl.addEventListener("click", async (e) => {
  const cancel = e.target.closest("[data-cancel]");
  if (cancel) {
    const id = cancel.dataset.cancel;
    // Not started yet: it has no process, so just drop it from our own queue.
    if (queue.isWaiting(id)) {
      finish(id, false, "Cancelled");
      return;
    }
    // Running: ask the backend to signal it. If it isn't running either, just
    // settle it here rather than leaving the row stuck.
    const killed = await invoke("cancel_download", { id }).catch(() => false);
    if (!killed) finish(id, false, "Cancelled");
    return;
  }

  const dismiss = e.target.closest("[data-dismiss]");
  if (dismiss) {
    queue.remove(dismiss.dataset.dismiss);
    renderQueue();
    syncCard(dismiss.dataset.dismiss);
    return;
  }

  const retry = e.target.closest("[data-retry]");
  if (retry) retryJob(retry.dataset.retry);
});

function retryJob(id) {
  if (!queue.retry(id)) return;
  renderQueue();
  syncCard(id);
  pump();
}

const retryFailed = () => Array.from(queue.jobs.entries())
  .filter(([, j]) => j.status === "error")
  .forEach(([id]) => retryJob(id));

$("qstop").addEventListener("click", async () => {
  // Resume all gets everything unfinished moving, the failed ones included.
  if ($("qstop").dataset.act !== "pause") {
    queue.resume();
    retryFailed();
    renderQueue();
    pump();
    return;
  }
  const id = queue.pause();
  renderQueue();
  if (!id) return;
  // The download's own report puts it back in line. With no process left to
  // report, that has to happen here.
  const killed = await invoke("cancel_download", { id }).catch(() => false);
  if (!killed) finish(id, false, "Cancelled");
});
$("clearq").addEventListener("click", async () => {
  const n = Array.from(queue.jobs.values()).filter((j) => j.status === "queued" || j.status === "error").length;
  const go = await ask(`This takes ${n} waiting or failed ${n === 1 ? "video" : "videos"} out of the queue. Anything already downloading carries on.`,
    { title: "Clear the queue?", kind: "warning", okLabel: "Clear queue", cancelLabel: "Cancel" });
  if (!go) return;
  queue.clearWaiting();
  queue.clearFailed();
  renderQueue();
  if (activeChannel()) renderList();
});

// Dragging a waiting row by its grip: a line shows where it would land, and
// letting go moves it there. Tracked by id, since a download finishing
// redraws the rows mid-drag.
let drag = null; // { id, before } — `before` is the row it would land ahead of, null for the end
const rowOf = (id) => queueEl.querySelector(`.qi[data-id="${CSS.escape(id)}"]`);
function markDrop() {
  queueEl.querySelectorAll(".dragging, .drop-before, .drop-after").forEach((el) => el.classList.remove("dragging", "drop-before", "drop-after"));
  if (!drag) return;
  rowOf(drag.id)?.classList.add("dragging");
  if (drag.before) rowOf(drag.before)?.classList.add("drop-before");
  else Array.from(queueEl.querySelectorAll('.qi[data-status="queued"]')).at(-1)?.classList.add("drop-after");
}
queueEl.addEventListener("pointerdown", (e) => {
  const grip = e.target.closest("[data-grip]");
  if (!grip || e.button !== 0) return;
  e.preventDefault();
  drag = { id: grip.closest(".qi").dataset.id, before: undefined };
  queueEl.classList.add("sorting");
});
window.addEventListener("pointermove", (e) => {
  if (!drag) return;
  if (!queue.isWaiting(drag.id)) { drag = null; queueEl.classList.remove("sorting"); return markDrop(); }
  const rows = Array.from(queueEl.querySelectorAll('.qi[data-status="queued"]'));
  const under = rows.find((r) => { const b = r.getBoundingClientRect(); return e.clientY < b.top + b.height / 2; });
  drag.before = under ? under.dataset.id : null;
  markDrop();
});
window.addEventListener("pointerup", () => {
  if (!drag) return;
  const { id, before } = drag;
  drag = null;
  queueEl.classList.remove("sorting");
  if (before !== undefined && queue.move(id, before)) renderQueue();
  else markDrop();
});

$("clear").addEventListener("click", () => {
  queue.clearFinished();
  renderQueue();
  if (activeChannel()) renderList();
});
// While a Sync runs, its button is greyed out and can't be pressed, and its
// arrow turns. When it's done the arrow finishes the turn it's on rather than
// stopping where it is, and the button comes back.
$("sync").addEventListener("click", async () => {
  const ch = activeChannel();
  const btn = $("sync");
  if (!ch) return;
  btn.classList.add("syncing");
  btn.disabled = true;
  try { await doFetch(ch.url, { quiet: true }); }
  finally {
    const arrow = btn.querySelector("svg");
    const stop = () => { btn.classList.remove("syncing"); btn.disabled = !activeChannel(); };
    // An arrow that never got to turn (reduced motion, say) has no turn to end.
    if (arrow.getAnimations().length) arrow.addEventListener("animationiteration", stop, { once: true });
    else stop();
  }
});

/* ---------------- cookie source picker ---------------- */
// Popover on the cookie button. Choosing a browser persists it in the state
// file and every later fetch/download hands it to yt-dlp. "None" turns the
// feature off again.
const cookieBtn = $("cookies");
const cookieMenu = document.createElement("div");
cookieMenu.className = "cookie-menu";
cookieMenu.innerHTML = `
  <div class="cookie-hd">Use cookies from</div>
  <button class="cookie-item" data-browser="">No cookies — anonymous</button>
  ${BROWSERS.map((b) => `<button class="cookie-item" data-browser="${b}">${b[0].toUpperCase() + b.slice(1)}</button>`).join("")}
  <div class="cookie-note">Log into YouTube in that browser first. Members-only videos need the browser with your channel membership.</div>`;
cookieBtn.parentElement.appendChild(cookieMenu);

function renderCookieState() {
  const on = !!state.cookies_browser;
  cookieBtn.classList.toggle("on", on);
  cookieMenu.querySelectorAll(".cookie-item").forEach((b) =>
    b.classList.toggle("active", b.dataset.browser === (state.cookies_browser || "")));
}
cookieBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  renderCookieState();
  cookieMenu.classList.toggle("open");
});
document.addEventListener("click", () => cookieMenu.classList.remove("open"));
cookieMenu.addEventListener("click", async (e) => {
  const item = e.target.closest(".cookie-item");
  if (!item) return;
  state.cookies_browser = item.dataset.browser || null;
  await persist();
  renderCookieState();
  cookieMenu.classList.remove("open");
  // Members-only cards unlock or lock with this choice.
  if (activeChannel()) renderList();
});

/* ---------------- tauri events ---------------- */
listen("download-progress", (e) => {
  const { id, downloaded: got, total, speed, eta } = e.payload;
  setJob(id, {
    status: "downloading",
    pct: total > 0 ? Math.min(100, (got / total) * 100) : 0,
    stat: [total > 0 && `${fmtBytes(got)} / ${fmtBytes(total)}`, speed > 0 && `${fmtBytes(speed)}/s`, eta > 0 && `ETA ${fmtDur(eta)}`]
      .filter(Boolean).join(" · ") || "Downloading…",
  });
});
listen("download-skipped", (e) => finish(e.payload.id, true, "Already in archive"));
listen("download-done", (e) => {
  const { id, ok, cancelled, reason } = e.payload;
  if (cancelled) finish(id, false, "Cancelled");
  else if (!ok && reason === BOT_CHECK) botCheck(id);
  else finish(id, ok, ok ? "Complete" : `Failed — ${reason || "unknown reason"}`);
});

// YouTube's bot check is waited out rather than answered with cookies, which
// tie the downloads to an account that can then be flagged. The line rests
// for each wait in turn; a download that gets through starts them over.
const BOT_CHECK = "YouTube bot check"; // the reason the backend gives it
const BOT_WAITS = [1, 5, 15, 30, 60].map((m) => m * 60_000);
let restTimer = null;
let restUntil = 0;
const clock = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
function botCheck(id) {
  // Still there after every wait: this row fails and the line holds, so the
  // rest don't each go and meet the check. Resume all tries again.
  if (queue.strikes >= BOT_WAITS.length) {
    queue.settle(id, false, `Failed — ${BOT_CHECK}, still there after ${BOT_WAITS.length + 1} tries`);
    queue.pause();
    renderQueue();
    syncCard(id);
    return;
  }
  const wait = BOT_WAITS[queue.rest(id) - 1];
  restUntil = Date.now() + wait;
  clearTimeout(restTimer);
  restTimer = setTimeout(() => { queue.wake(); renderQueue(); pump(); }, wait);
  renderQueue();
  syncCard(id);
}

// Marks a column that has nothing to scroll, so it is drawn without a
// scrollbar; see .still in the stylesheet.
for (const el of [document.querySelector(".main"), queueEl]) {
  let due = false;
  const sync = () => {
    if (due) return;
    due = true;
    requestAnimationFrame(() => { due = false; el.classList.toggle("still", el.scrollHeight <= el.clientHeight + 1); });
  };
  const sized = new ResizeObserver(sync);
  sized.observe(el);
  for (const child of el.children) sized.observe(child);
  new MutationObserver(sync).observe(el, { childList: true, subtree: true });
  sync();
}

/* ---------------- boot ---------------- */
$("fetch").addEventListener("click", () => doFetch($("url").value));
$("url").addEventListener("keydown", (e) => { if (e.key === "Enter") doFetch($("url").value); });
// The add box clears the way the channel's search box does: its ✕, or Esc,
// and a second Esc leaves it.
function setUrl(text) {
  $("url").value = text;
  $("url-x").hidden = !text;
}
$("url").addEventListener("input", () => setUrl($("url").value));
$("url-x").addEventListener("click", (e) => { e.preventDefault(); setUrl(""); $("url").focus(); });
$("url").addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  e.stopPropagation();
  if ($("url").value) setUrl(""); else $("url").blur();
});

async function boot() {
  try { state = await invoke("load_state"); }
  catch (e) {
    stateError = String(e);
    setEmpty(`${MARK}<h2>Your saved channels couldn't be read</h2><p>${esc(stateError)}</p>`);
    $("url").disabled = true;
    $("fetch").disabled = true;
    return;
  }
  // Splits the old global archive into per-channel ones. Idempotent and
  // guarded on the Rust side, so it's safe to call on every launch.
  try {
    const m = await invoke("migrate_archives");
    if (m.ran && (m.moved || m.unmatched)) {
      console.info(`Archive migration: ${m.moved} ids into ${m.channels} channel folder(s)` +
        (m.unmatched ? `, ${m.unmatched} left unattributed` : ""));
    }
  } catch (e) { console.error("Archive migration failed:", e); }
  await loadArchive();
  renderSelector();
  renderCookieState();
  showActive();
  renderQueue();
  pump(); // whatever was still in line when Vault last closed
  renderWatching();
  checkSubscriptions();
}
boot();
