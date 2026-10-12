import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, ask } from "@tauri-apps/plugin-dialog";
import { createQueue } from "./queue.js";
import { isMembers, archiveState, view, downloadable, withUnlisted, batchSizes, ASK_ABOVE } from "./library.js";

/* ---------------- theme ---------------- */
const root = document.documentElement;
const savedTheme = localStorage.getItem("vault-theme");
if (savedTheme) root.setAttribute("data-theme", savedTheme);
document.getElementById("theme").addEventListener("click", () => {
  const next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
  root.setAttribute("data-theme", next);
  localStorage.setItem("vault-theme", next);
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
const queue = createQueue(); // the download jobs and their order — see queue.js

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
  if (!quiet) setEmpty(`<div class="mark">VAULT</div><h2>Fetching…</h2><p>Reading the channel's video list.</p><div class="spin"></div>`);
  $("fetch").disabled = true;
  try {
    const info = await invoke("fetch_channel", { url: raw, tab: "videos", cookies: state.cookies_browser ?? null });
    if (!info.videos || !info.videos.length) {
      if (!quiet) setEmpty(`<div class="mark">VAULT</div><h2>No videos found</h2><p>That returned nothing. Check the URL or @handle and try again.</p>`);
      else alert(`Couldn't refresh ${raw}.\n\nYouTube returned no videos. The saved list is unchanged — try again in a moment.`);
      return;
    }
    let ch = state.channels.find((c) => c.url === info.url);
    if (!ch) {
      ch = { url: info.url, title: info.title, uploader: info.uploader, video_count: 0, fetched_at: 0, download_dir: null, videos: [] };
      state.channels.unshift(ch);
    }
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
  } catch (e) {
    if (!quiet) setEmpty(`<div class="mark">VAULT</div><h2>Couldn't fetch that</h2><p>${esc(String(e))}</p>`);
    // Sync keeps the list on screen, so its failure has nowhere to show. Say
    // it rather than leave the click looking ignored.
    else alert(`Couldn't refresh ${raw}.\n\n${e}\n\nThe saved list is unchanged.`);
  } finally {
    $("fetch").disabled = false;
  }
}

function showActive() {
  const ch = activeChannel();
  if (!ch) {
    setEmpty(`<div class="mark">VAULT</div><h2>Fetch a channel</h2><p>Paste a channel URL or type an @handle. Vault lists every video and keeps the ones you choose.</p>`);
    $("folder").disabled = true;
    $("sync").disabled = true;
    return;
  }
  rebuildArchived();
  $("folder").disabled = false;
  $("sync").disabled = false;
  // The field is only for adding a channel; Sync refreshes the one on screen.
  $("url").value = "";
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
  const handle = ch.url.split("/").pop();
  const meta = [
    handle.startsWith("@") && esc(handle),
    ch.followers != null && `${fmtViews(ch.followers)} subscribers`,
    `${ch.video_count} videos`,
  ].filter(Boolean).join(" · ");
  const dir = ch.download_dir
    ? `<div class="dirmeta">${FOLDER_ICON}
         <a class="path" id="open-dir" title="Open this folder">${esc(ch.download_dir)}</a>
         <button class="mini" id="change-dir">Change</button>
       </div>`
    : `<div class="dirmeta">${FOLDER_ICON}
         <span>No download folder yet — you'll be asked when you download the first video.</span>
         <button class="mini accent" id="change-dir">Choose folder</button>
       </div>`;

  headEl.innerHTML = `
    ${ch.banner ? `<img class="banner" src="${esc(ch.banner)}" alt="">` : ""}
    <div class="chan">
      <div class="avatar">${ch.avatar ? `<img src="${esc(ch.avatar)}" alt="">` : esc(initial)}</div>
      <div class="chan-info">
        <h1>${esc(ch.title)}${ch.verified ? `<span class="tick" title="Verified">✓</span>` : ""}</h1>
        <div class="chan-meta">${meta}</div>
        ${dir}
      </div>
    </div>`;

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
  const list = ch && tabVideos(ch, activeTab);
  return list ? view(list, filters, archived) : null;
}

// Controls and grid always redraw together: every number in the controls is
// derived from the same list the grid shows.
function renderList() {
  const now = currentView();
  renderControls(now);
  renderGrid(now);
}

function renderControls(now) {
  const ch = activeChannel();
  if (!ch) { controlsEl.innerHTML = ""; return; }

  let tabs = Object.entries(TABS).map(([t, label]) =>
    `<button class="tab ${activeTab === t ? "active" : ""}" data-tab="${t}">${label}</button>`).join("");

  // One choice per group, like the tabs. Each number is what that choice
  // would show, so the two selected ones always match the grid.
  const group = (name, labels, counts) => `<div class="filters">${Object.entries(labels).map(([value, label]) =>
    `<button class="pill ${filters[name] === value ? "active" : ""}" data-filter="${name}" data-value="${value}">${label} <span class="n">${counts[value]}</span></button>`).join("")}</div>`;

  // The filters and sort sit on one row; the download button rides the tab
  // strip, where there is room, so the row doesn't wrap at the default width.
  let bar = "";
  if (now) {
    const { jobs, locked } = downloadable(now.shown, archived, { cookies: !!state.cookies_browser, holds: queue.holds });
    // With nothing to queue, say why instead of offering "Download 0".
    // The arrow offers the first few in the order on screen, so the sort
    // picks the batch: Oldest + First 50 is the 50 oldest not yet archived.
    const sizes = batchSizes(jobs.length);
    const menu = sizes.length ? `<button class="btn btn-primary split-arrow" id="dlmore" title="Download only the first few">▾</button>
        <div class="dl-menu" id="dlmenu">
          ${sizes.map((n) => `<button class="cookie-item" data-first="${n}">First ${n}</button>`).join("")}
          <button class="cookie-item" data-first="${jobs.length}">All ${jobs.length}</button>
        </div>` : "";
    const download = jobs.length ? `<div class="split"><button class="btn btn-primary" id="dlshown">↓ Download ${jobs.length} shown</button>${menu}</div>`
      : locked ? `<button class="btn btn-primary" id="dlshown" title="These are members-only. Pick the browser you're signed into YouTube with.">Set cookies to download</button>`
      : `<button class="btn btn-primary" id="dlshown" disabled>Nothing to download</button>`;
    bar = `
      ${now.hasMembers ? group("access", ACCESS, now.counts.access) : ""}
      ${group("have", HAVE, now.counts.have)}
      <span class="tabs-spacer"></span>
      <select id="sort">${Object.entries(SORTS).map(([value, label]) =>
        `<option value="${value}"${filters.sort === value ? " selected" : ""}>${label}</option>`).join("")}</select>`;
    tabs += `<span class="tabs-spacer"></span>${download}`;
  }

  controlsEl.innerHTML = `<div class="tabs">${tabs}</div><div class="controls">${bar}</div>`;

  controlsEl.querySelectorAll("[data-tab]").forEach((b) =>
    b.addEventListener("click", () => switchTab(b.dataset.tab)));
  controlsEl.querySelectorAll("[data-filter]").forEach((b) =>
    b.addEventListener("click", () => setFilter(b.dataset.filter, b.dataset.value)));
  $("sort")?.addEventListener("change", (e) => setFilter("sort", e.target.value));
  $("dlshown")?.addEventListener("click", () => downloadShown());
  $("dlmore")?.addEventListener("click", (e) => {
    e.stopPropagation();
    $("dlmenu").classList.toggle("open");
  });
  controlsEl.querySelectorAll("[data-first]").forEach((b) =>
    b.addEventListener("click", () => downloadShown(Number(b.dataset.first))));
}
document.addEventListener("click", () => $("dlmenu")?.classList.remove("open"));

function setFilter(name, value) {
  filters[name] = value;
  localStorage.setItem(`vault-${name}`, value);
  renderList();
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
    emptyEl.innerHTML = `<div class="mark">VAULT</div><h2>${title}</h2>${body}`;
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
  if (!now.shown.length) {
    const unfiltered = filters.access === "all" && filters.have === "all";
    message("Nothing here", `<p>${unfiltered
      ? `No ${TABS[activeTab].toLowerCase()} on this channel.`
      : "No videos match these filters."}</p>`);
    return;
  }

  emptyEl.style.display = "none";
  gridEl.style.display = "grid";
  gridVideos = new Map(now.shown.map((v) => [v.id, v]));
  gridEl.innerHTML = now.shown.map(card).join("");
}

function card(v) {
  const entry = archived.get(v.id);
  const have = archiveState(v.id, archived);
  const playable = have === "have" && !!entry?.path;
  const meta = [
    fmtViews(v.view_count) && `${fmtViews(v.view_count)} views`,
    v.unlisted && "Missing on YT",
  ].filter(Boolean).join(" · ");
  return `
    <div class="card" data-id="${esc(v.id)}">
      <div class="thumb ${playable ? "playable" : ""}" ${playable ? `data-act="play" title="Play ${esc(v.title)}"` : ""}>
        <img loading="lazy" src="${thumb(v.id)}" alt="">
        ${fmtDur(v.duration) ? `<span class="dur">${fmtDur(v.duration)}</span>` : ""}
        ${isMembers(v) ? `<span class="badge">Members only</span>` : ""}
        ${have === "missing" ? `<span class="badge gone">File missing</span>` : ""}
        ${playable ? '<span class="play">▶</span>' : ""}
      </div>
      <div class="card-body">
        <h3>${esc(v.title)}</h3>
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
      ? `<button class="dl on" data-id="${id}" data-act="play">▶ Play</button>`
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
  if (target.dataset.act === "play") openInPlayer(archived.get(id)?.path);
  else if (gridVideos.has(id)) startDownload(gridVideos.get(id));
});

/* ---------------- playback ---------------- */
// Playback is handed to the system: whatever app owns the extension opens it,
// which on a Mac set up for archiving is VLC. An in-app <video> was tried and
// removed — the webview can't be granted access to the volumes the library
// lives on, and a player that can't read the file is worse than none.
//
// `open` resolves the app through LaunchServices, so this stays out of the
// business of knowing which player is installed.
function openInPlayer(path) {
  if (!path) return;
  invoke("open_file", { path }).catch((err) => {
    console.error("Could not open:", err);
    // A silent failure here is the worst outcome: the click looks ignored.
    alert(`Could not open this file.\n\n${err}\n\nThe file is at:\n${path}`);
  });
}

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
// Where a video's file goes, and whether to force it. A file the archive
// records but the disk has lost goes back to the folder that recorded it, in
// force mode: yt-dlp skips it otherwise, since the archive still lists the id,
// and the backend re-stamps the archive only after the file lands.
function jobFor(v, dir) {
  const missing = archiveState(v.id, archived) === "missing";
  return { id: v.id, title: v.title, url: v.url, dir: (missing && archived.get(v.id).dir) || dir, force: missing };
}

async function startDownload(v) {
  const ch = activeChannel();
  if (!ch || queue.holds(v.id)) return;
  const dir = await ensureDir(ch);
  if (!dir) return;
  if (!queue.add(jobFor(v, dir))) return;
  renderQueue();
  syncCard(v.id);
  renderControls(currentView()); // one fewer for "Download N shown"
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
      `This adds ${batch.length} videos to the queue. They download one at a time, so it can run for hours and take a lot of disk space. The arrow next to the button queues a smaller batch.`,
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
  invoke("download_video", { id: job.id, url: job.url, dir: job.dir, force: job.force, cookies: state.cookies_browser ?? null })
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
    if (activeChannel()) renderList();
  }, 600);
}

/* ---------------- queue rendering ---------------- */
const queueEl = $("queue");

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
      <img src="${thumb(id)}" alt="">
      <div class="m">
        <div class="t">${esc(j.title)}</div>
        <div class="bar"><i class="${j.status === "done" ? "done" : ""}" style="width:${j.pct}%"></i></div>
        <div class="stat">
          <span class="grow ${j.status === "error" ? "reason" : ""}" title="${esc(j.stat)}">${esc(j.stat)}</span>
          ${controls}
        </div>
      </div>
    </div>`;
  }).join("");
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
  g.textContent = j.stat;
  g.title = j.stat;
  g.className = "grow" + (j.status === "error" ? " reason" : "");
}

function renderSummary() {
  const list = Array.from(queue.jobs.values());
  const down = list.filter((j) => j.status === "downloading").length;
  const q = list.filter((j) => j.status === "queued").length;
  const done = list.filter((j) => j.status === "done").length;
  const err = list.filter((j) => j.status === "error").length;
  $("qsum").textContent = queue.jobs.size === 0
    ? "Idle"
    : [`${down} downloading`, `${q} queued`, done && `${done} done`, err && `${err} failed`]
        .filter(Boolean).join(" · ");
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

$("clear").addEventListener("click", () => {
  queue.clearFinished();
  renderQueue();
  if (activeChannel()) renderList();
});
$("retryall").addEventListener("click", () => {
  Array.from(queue.jobs.entries())
    .filter(([, j]) => j.status === "error")
    .forEach(([id]) => retryJob(id));
});
$("folder").addEventListener("click", () => { const ch = activeChannel(); if (ch) changeDir(ch); });
$("sync").addEventListener("click", () => {
  const ch = activeChannel();
  if (ch) doFetch(ch.url, { quiet: true });
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
  else finish(id, ok, ok ? "Complete" : `Failed — ${reason || "unknown reason"}`);
});

/* ---------------- boot ---------------- */
$("fetch").addEventListener("click", () => doFetch($("url").value));
$("url").addEventListener("keydown", (e) => { if (e.key === "Enter") doFetch($("url").value); });

async function boot() {
  try { state = await invoke("load_state"); }
  catch (e) {
    stateError = String(e);
    setEmpty(`<div class="mark">VAULT</div><h2>Your saved channels couldn't be read</h2><p>${esc(stateError)}</p>`);
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
}
boot();
