import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

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
const isMembers = (v) => v.availability === "subscriber_only";

/* ---------------- state ---------------- */
let state = { active: null, channels: [] };
let downloaded = new Set(); // ids archived in the ACTIVE channel's vault
const jobs = new Map();   // id -> { title, thumb, url, dir, pct, status, stat, reason }
let pending = [];         // queued { id, url, dir }
let activeId = null;      // the job currently downloading (concurrency = 1)

// Shorts and the live tab are fetched on demand and kept in memory only —
// the Videos tab is the one that persists.
const tabCache = new Map(); // channelUrl -> { shorts: [...], live: [...] }

// The Archive tab spans every channel, not just the active one, so it lives
// outside tabCache. Rebuilt on demand (boot, after a download finishes, on
// sync) rather than persisted — the archive files are the source of truth.
let archive = null; // null = not loaded yet
let vaults = [];    // per-channel archive state, for the header note

let activeTab = localStorage.getItem("vault-tab") || "videos";
let sort = localStorage.getItem("vault-sort") || "newest";
let filter = localStorage.getItem("vault-filter") || "all";

const activeChannel = () => state.channels.find((c) => c.url === state.active) || null;

async function loadArchive() {
  try {
    archive = await invoke("list_archive");
  } catch (e) {
    archive = [];
    console.error("Could not read the archive:", e);
  }
  try {
    vaults = await invoke("channel_vaults");
  } catch (e) {
    vaults = [];
    console.error("Could not read channel vaults:", e);
  }
  rebuildDownloaded();
  return archive;
}

// Per-channel archive state: an id counts as downloaded only for the channel
// whose folder records it, so two channels that share a video don't mark each
// other's copy. Rebuilt whenever the active channel or the archive changes.
function rebuildDownloaded() {
  downloaded = new Set(scopedArchive().map((e) => e.id));
}

// The archive entries owned by the active channel — the channel whose folder
// recorded them. Ownership comes from the archive files, so a video that has
// dropped off the channel's fetched list (deleted on YouTube) still shows.
function scopedArchive() {
  return (archive || []).filter((e) => e.channel_url === state.active);
}

async function persist() {
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
    const info = await invoke("fetch_channel", { url: raw, tab: "videos" });
    if (!info.videos || !info.videos.length) {
      if (!quiet) setEmpty(`<div class="mark">VAULT</div><h2>No videos found</h2><p>That returned nothing. Check the URL or @handle and try again.</p>`);
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
    ch.fetched_at = Math.floor(Date.now() / 1000);
    state.active = info.url;
    tabCache.delete(info.url); // tab caches are stale after a re-fetch
    activeTab = "videos";
    await persist();
    renderSelector();
    showActive();
  } catch (e) {
    if (!quiet) setEmpty(`<div class="mark">VAULT</div><h2>Couldn't fetch that</h2><p>${esc(String(e))}</p>`);
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
  rebuildDownloaded();
  $("folder").disabled = false;
  $("sync").disabled = false;
  $("url").value = ch.url;
  renderHeader(ch);
  renderControls();
  renderGrid();
}

function renderHeader(ch) {
  const initial = (ch.title || "?").trim().charAt(0).toUpperCase();
  // The chip names the folder this channel's videos and vault live in. The
  // vault is a hidden .vault/ inside it, mentioned here so the per-channel
  // layout is legible from the UI rather than only from the filesystem.
  const vault = vaults.find((v) => v.url === ch.url);
  const vaultNote = vault && ch.download_dir
    ? vault.reachable
      ? `vault · ${vault.archived} archived`
      : `vault unreachable`
    : "";
  const dirChip = ch.download_dir
    ? `<div class="dirmeta">
         <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
         <span class="path">${esc(ch.download_dir)}</span>
         ${vaultNote ? `<span class="vaultnote ${vault.reachable ? "" : "bad"}">${vaultNote}</span>` : ""}
         <button class="mini" id="open-dir">Open</button>
         <button class="mini" id="change-dir">Change</button>
       </div>`
    : `<div class="dirmeta">
         <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
         <span>No download folder yet — you'll be asked when you download the first video.</span>
         <button class="mini accent" id="change-dir">Choose folder</button>
       </div>`;

  const members = ch.videos.filter(isMembers).length;
  headEl.innerHTML = `
    <div class="chan">
      <div class="avatar">${esc(initial)}</div>
      <div class="chan-info">
        <h1>${esc(ch.title)}</h1>
        <div class="chan-meta"><b>${ch.video_count}</b> videos · <b>${members}</b> members-only · <b>${downloaded.size}</b> in archive</div>
      </div>
      <div class="chan-actions">
        <button class="btn btn-primary" id="dlall">Download all</button>
      </div>
    </div>
    ${dirChip}`;

  $("dlall").addEventListener("click", downloadAll);
  $("change-dir")?.addEventListener("click", () => changeDir(ch));
  $("open-dir")?.addEventListener("click", () => invoke("reveal_folder", { dir: ch.download_dir }).catch(() => {}));
}

/* ---------------- tabs / sort / filter ---------------- */
const TAB_LABEL = { videos: "Videos", shorts: "Shorts", streams: "Live", archive: "Archive" };
const tabKey = (tab) => (tab === "streams" ? "live" : "shorts");

function tabVideos(ch, tab) {
  if (tab === "videos") return ch.videos;
  if (tab === "archive") return scopedArchive(); // this channel's vault, or all
  return tabCache.get(ch.url)?.[tabKey(tab)] || null; // null = not fetched yet
}

// The Archive tab is the only one that isn't confined to one channel.
const isArchive = () => activeTab === "archive";

function renderControls() {
  const ch = activeChannel();
  if (!ch) { controlsEl.innerHTML = ""; return; }
  const members = ch.videos.filter(isMembers).length;
  const pub = ch.videos.length - members;

  const tabBtn = (t) => {
    const list = tabVideos(ch, t);
    const n = list ? `<span class="n">${list.length}</span>` : "";
    return `<button class="tab ${activeTab === t ? "active" : ""}" data-tab="${t}">${TAB_LABEL[t]}${n}</button>`;
  };

  // Every option carries a count, including All, so the two menus read the
  // same way.
  const total = (tabVideos(ch, activeTab) || []).length;
  const onDisk = isArchive() ? scopedArchive().filter((v) => v.path).length : 0;
  const filters = isArchive()
    ? `<div class="filters">
         <button class="pill ${filter === "all" ? "active" : ""}" data-filter="all">All <span class="n">${total}</span></button>
         <button class="pill ${filter === "on-disk" ? "active" : ""}" data-filter="on-disk">On disk <span class="n">${onDisk}</span></button>
         <button class="pill ${filter === "missing" ? "active" : ""}" data-filter="missing">Not found <span class="n">${total - onDisk}</span></button>
       </div>`
    : `<div class="filters">
         <button class="pill ${filter === "all" ? "active" : ""}" data-filter="all">All <span class="n">${ch.videos.length}</span></button>
         <button class="pill ${filter === "public" ? "active" : ""}" data-filter="public">Public <span class="n">${pub}</span></button>
         <button class="pill ${filter === "members" ? "active" : ""}" data-filter="members">Members <span class="n">${members}</span></button>
       </div>`;

  // The Archive tab always shows the channel selected at the top — the channel
  // selector is the one picker in the app, and per-channel is the whole point.

  controlsEl.innerHTML = `
    <div class="controls">
      <div class="tabs">${tabBtn("videos")}${tabBtn("shorts")}${tabBtn("streams")}${tabBtn("archive")}</div>
      ${filters}
      <select id="sort">
        <option value="newest"${sort === "newest" ? " selected" : ""}>Newest</option>
        <option value="oldest"${sort === "oldest" ? " selected" : ""}>Oldest</option>
        <option value="popular"${sort === "popular" ? " selected" : ""}>Most popular</option>
        <option value="shortest"${sort === "shortest" ? " selected" : ""}>Shortest</option>
        <option value="longest"${sort === "longest" ? " selected" : ""}>Longest</option>
      </select>
      <span class="shown" id="shown"></span>
    </div>`;

  controlsEl.querySelectorAll("[data-tab]").forEach((b) =>
    b.addEventListener("click", () => switchTab(b.dataset.tab)));
  controlsEl.querySelectorAll("[data-filter]").forEach((b) =>
    b.addEventListener("click", () => {
      filter = b.dataset.filter;
      localStorage.setItem("vault-filter", filter);
      renderControls(); renderGrid();
    }));
  $("sort").addEventListener("change", (e) => {
    sort = e.target.value;
    localStorage.setItem("vault-sort", sort);
    renderGrid();
  });
}

async function switchTab(tab) {
  const ch = activeChannel();
  if (!ch || tab === activeTab) return;
  activeTab = tab;
  localStorage.setItem("vault-tab", tab);
  // The live/members filters only apply to a channel's own tabs. Passing them
  // into the Archive would show "Nothing here" for an archive that isn't empty.
  if (isArchive() ? filter === "public" || filter === "members" : filter === "on-disk" || filter === "missing") {
    filter = "all";
    localStorage.setItem("vault-filter", filter);
  }

  if (tab === "archive") {
    // A channel's vault lives in its folder, so there is nothing to show until
    // one is chosen. Ask the same way the first download does, rather than
    // showing an empty archive that looks like a bug.
    if (!ch.download_dir) {
      renderControls();
      controlsEl.innerHTML = `<div class="controls"><span class="shown">This channel has no folder yet.</span></div>`;
      setEmpty(`<div class="mark">VAULT</div><h2>No folder yet</h2>
        <p>${esc(ch.title)} downloads nowhere yet, so it has no archive. Choose a folder to start one.</p>
        <div style="margin-top:18px"><button class="btn btn-primary" id="archive-pick">Choose folder</button></div>`);
      $("archive-pick")?.addEventListener("click", async () => {
        const dir = await ensureDir(ch);
        if (dir) { await loadArchive(); showActive(); }
      });
      return;
    }
    controlsEl.innerHTML = `<div class="controls"><div class="spin"></div><span class="shown">Reading the archive…</span></div>`;
    await loadArchive();
  } else if (tab !== "videos" && !tabVideos(ch, tab)) {
    controlsEl.innerHTML = `<div class="controls"><div class="spin"></div><span class="shown">Loading ${TAB_LABEL[tab]}…</span></div>`;
    try {
      const info = await invoke("fetch_channel", { url: ch.url, tab });
      const entry = tabCache.get(ch.url) || {};
      entry[tabKey(tab)] = info.videos || [];
      tabCache.set(ch.url, entry);
    } catch (e) {
      const entry = tabCache.get(ch.url) || {};
      entry[tabKey(tab)] = [];
      tabCache.set(ch.url, entry);
      console.error(`Could not load ${tab}:`, e);
    }
  }
  renderControls();
  renderGrid();
}

function visibleVideos() {
  const ch = activeChannel();
  if (!ch) return [];
  let list = tabVideos(ch, activeTab) || [];
  list = list.slice();
  if (isArchive()) {
    if (filter === "on-disk") list = list.filter((v) => v.path);
    else if (filter === "missing") list = list.filter((v) => !v.path);
  } else if (filter === "members") list = list.filter(isMembers);
  else if (filter === "public") list = list.filter((v) => !isMembers(v));

  // Ascending by duration, shortest first. Videos with no duration (shorts,
  // which carry none in flat mode) go last either way.
  switch (sort) {
    case "oldest": list.reverse(); break;
    case "popular": list.sort((a, b) => (b.view_count ?? -1) - (a.view_count ?? -1)); break;
    case "shortest": list.sort((a, b) => (a.duration ?? Infinity) - (b.duration ?? Infinity)); break;
    case "longest": list.sort((a, b) => (b.duration ?? -1) - (a.duration ?? -1)); break;
    // "newest" is the order YouTube returns, which is already newest-first.
    // No upload date is available in flat mode, so it can't be sorted on.
  }
  return list;
}

function renderGrid() {
  const ch = activeChannel();
  if (!ch) return;
  const list = visibleVideos();
  const total = (tabVideos(ch, activeTab) || []).length;
  emptyEl.style.display = "none";
  gridEl.style.display = "grid";
  const shown = $("shown");
  if (shown) shown.textContent = list.length === total ? `${total} shown` : `${list.length} of ${total} shown`;

  if (!list.length) {
    gridEl.style.display = "none";
    emptyEl.style.display = "flex";
    emptyEl.innerHTML = `<div class="mark">VAULT</div><h2>Nothing here</h2><p>${
      total === 0
        ? isArchive() ? "Nothing archived yet. Download a video and it lands here." : `No ${TAB_LABEL[activeTab].toLowerCase()} on this channel.`
        : "No videos match this filter."
    }</p>`;
    return;
  }

  gridEl.innerHTML = list.map((v) => isArchive() ? archiveCard(v) : videoCard(v)).join("");
}

// A card for the Videos / Shorts / Live tabs. `v` is a channel video.
function videoCard(v) {
  const done = downloaded.has(v.id);
  const member = isMembers(v);
  const meta = [fmtViews(v.view_count) && `${fmtViews(v.view_count)} views`, fmtDur(v.duration)]
    .filter(Boolean).join(" · ");
  const label = member ? "Members only" : done ? "✓ In archive" : "↓ Download";
  return `
    <div class="card">
      <div class="thumb">
        <img loading="lazy" src="${thumb(v.id)}" alt="">
        ${fmtDur(v.duration) ? `<span class="dur">${fmtDur(v.duration)}</span>` : ""}
        ${member ? `<span class="badge">Members only</span>` : ""}
      </div>
      <div class="card-body">
        <h3>${esc(v.title)}</h3>
        <div class="sub">${meta || "&nbsp;"}</div>
        <div class="card-foot">
          <button class="dl ${done && !member ? "on" : ""}" data-id="${v.id}"
            ${member ? 'disabled title="This video is for channel members only — yt-dlp without your cookies cannot fetch it."' : ""}>
            ${label}
          </button>
        </div>
      </div>
    </div>`;
}

// A card for the Archive tab. `e` is an archive entry. Thumbnails open the file
// when it's on disk, so there's no need to print the path on the card.
function archiveCard(e) {
  const onDisk = !!e.path;
  const meta = [e.channel, fmtViews(e.view_count) && `${fmtViews(e.view_count)} views`, fmtDur(e.duration)]
    .filter(Boolean).join(" · ");
  const thumbEl = `
    <div class="thumb ${onDisk ? "playable" : "inert"}"
         ${onDisk ? `data-open="${esc(e.path)}" title="Play ${esc(e.title)}"` : `title="${e.dir
           ? `Not in ${esc(e.dir)} — re-download to restore it.`
           : "This channel's folder isn't known to Vault, so the file can't be located."}"`}>
      <img loading="lazy" src="${thumb(e.id)}" alt="">
      ${fmtDur(e.duration) ? `<span class="dur">${fmtDur(e.duration)}</span>` : ""}
      ${onDisk ? '<span class="play">▶</span>' : '<span class="badge gone">Not found</span>'}
    </div>`;
  const foot = onDisk
    ? `<button class="dl on" data-open="${esc(e.path)}">▶ Play</button>`
    // Re-download needs to know where to put the file, which only works when
    // the owning channel's folder is known. The forced run ignores the archive
    // (yt-dlp would skip an id it still records) and re-stamps it on success.
    : e.dir
      ? `<button class="dl" data-redl="${e.id}">↻ Download again</button>`
      : `<button class="dl" disabled title="Vault doesn't know which folder held this file, so it can't be re-downloaded.">File not found</button>`;
  return `
    <div class="card">
      ${thumbEl}
      <div class="card-body">
        <h3>${esc(e.title)}</h3>
        <div class="sub">${meta || esc(e.id)}</div>
        <div class="card-foot">${foot}</div>
      </div>
    </div>`;
}

gridEl.addEventListener("click", (e) => {
  const o = e.target.closest("[data-open]");
  if (o) { openInPlayer(o.dataset.open); return; }
  const r = e.target.closest("[data-redl]");
  if (r) { redownload(r.dataset.redl); return; }
  const b = e.target.closest(".dl");
  if (!b || b.disabled) return;
  const ch = activeChannel();
  const v = ch?.videos.find((x) => x.id === b.dataset.id);
  if (v) enqueue(v);
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
  renderHeader(ch);
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
  renderHeader(ch);
}

/* ---------------- queue engine (one at a time) ---------------- */
const queuedOrRunning = (id) => jobs.has(id) && jobs.get(id).status !== "error";

async function enqueue(v) {
  const ch = activeChannel();
  if (!ch || queuedOrRunning(v.id) || pending.some((p) => p.id === v.id)) return;
  const dir = await ensureDir(ch);
  if (!dir) return;
  pending.push({ id: v.id, url: v.url, dir, title: v.title });
  addJob(v.id, v.title, v.url, dir, "queued", "Queued");
  pump();
}

async function downloadAll() {
  const ch = activeChannel();
  if (!ch) return;
  const dir = await ensureDir(ch);
  if (!dir) return;
  let skipped = 0;
  for (const v of ch.videos) {
    if (isMembers(v)) { skipped++; continue; } // members-only will always fail
    if (downloaded.has(v.id) || queuedOrRunning(v.id) || pending.some((p) => p.id === v.id)) continue;
    pending.push({ id: v.id, url: v.url, dir, title: v.title });
    addJob(v.id, v.title, v.url, dir, "queued", "Queued");
  }
  if (skipped) console.info(`Skipped ${skipped} members-only video(s).`);
  pump();
}

// Re-fetch a file that the archive records but the disk doesn't. The job runs
// in force mode: yt-dlp skips it otherwise, since the archive still lists the
// id, and the backend re-stamps the archive only after the file lands. A
// failure leaves the archive untouched, so the entry stays visible here.
function redownload(id) {
  const entry = (archive || []).find((e) => e.id === id);
  if (!entry || !entry.dir) return;
  if (queuedOrRunning(id) || pending.some((p) => p.id === id)) return;
  const title = entry.title || id;
  pending.push({ id, url: entry.url, dir: entry.dir, title, force: true });
  addJob(id, title, entry.url, entry.dir, "queued", "Queued");
  pump();
}

function pump() {
  if (activeId || pending.length === 0) return;
  const next = pending.shift();
  activeId = next.id;
  setJob(next.id, { status: "downloading", stat: "Starting…", pct: 0 });
  invoke("download_video", { id: next.id, url: next.url, dir: next.dir, force: next.force || null })
    .catch((e) => finish(next.id, false, `Error: ${e}`));
}

function addJob(id, title, url, dir, status, stat, force = false) {
  if (!jobs.has(id)) jobs.set(id, { title, thumb: thumb(id), url, dir, pct: 0, status, stat, reason: null, force });
  renderQueue();
}
function setJob(id, patch) {
  const j = jobs.get(id);
  if (!j) return;
  Object.assign(j, patch);
  updateJobElement(id);
  renderSummary();
  syncCard(id);
}
function finish(id, ok, stat) {
  const j = jobs.get(id);
  if (!j) return;
  j.status = ok ? "done" : "error";
  j.pct = ok ? 100 : j.pct;
  j.stat = stat || (ok ? "Complete" : "Failed");
  j.reason = ok ? null : stat;
  // Marks the card immediately, but only when the file landed in the active
  // channel's folder — the archive is per channel. A job that finished for a
  // channel the user has already navigated away from is corrected by the
  // archive reload below, which rebuilds the set from the files themselves.
  if (ok) {
    const ch = activeChannel();
    if (ch && j.dir && ch.download_dir === j.dir) downloaded.add(id);
  }
  if (activeId === id) activeId = null;
  updateJobElement(id);
  renderSummary();
  syncCard(id);
  pump();
  // A finished download joins the archive, so the tab would be stale. Refresh
  // it lazily: only if it's showing, or the user is about to see it next time.
  if (ok) refreshArchiveSoon();
}

// Coalesces the burst of archive reads that a `Download all` run would
// otherwise trigger — one reload shortly after the last completion instead of
// one per video.
let archiveRefresh = null;
function refreshArchiveSoon() {
  if (archiveRefresh) clearTimeout(archiveRefresh);
  archiveRefresh = setTimeout(async () => {
    archiveRefresh = null;
    await loadArchive();
    if (isArchive()) { renderControls(); renderGrid(); }
  }, 600);
}

function syncCard(id) {
  const b = gridEl.querySelector(`.dl[data-id="${CSS.escape(id)}"]`);
  if (!b || b.disabled) return;
  const j = jobs.get(id);
  // Progress outranks the badge: a re-download of an id that is still in the
  // archive must show its percentage, not "✓ In archive".
  if (j && j.status === "downloading") { b.classList.add("on"); b.textContent = `↓ ${Math.round(j.pct)}%`; }
  else if (j && j.status === "queued") { b.classList.add("on"); b.textContent = "• Queued"; }
  else if (downloaded.has(id)) { b.classList.add("on"); b.textContent = "✓ In archive"; }
  else if (j && j.status === "error") { b.classList.remove("on"); b.textContent = "↻ Retry"; }
  else { b.classList.remove("on"); b.textContent = "↓ Download"; }
}

/* ---------------- queue rendering ---------------- */
const queueEl = $("queue");

function renderQueue() {
  if (jobs.size === 0) {
    queueEl.innerHTML = `<div class="qempty">Nothing queued yet.</div>`;
    renderSummary();
    return;
  }
  queueEl.innerHTML = Array.from(jobs.entries()).map(([id, j]) => {
    const controls =
      j.status === "done" ? '<span class="okmark">✓</span>'
      : j.status === "error"
        ? `<button class="qbtn retry" data-retry="${id}">⊘ Retry</button><button class="x" data-dismiss="${id}">✕</button>`
        : `<button class="x" data-cancel="${id}">✕</button>`;
    return `
    <div class="qi" data-id="${id}">
      <img src="${j.thumb}" alt="">
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
  const j = jobs.get(id);
  const el = queueEl.querySelector(`.qi[data-id="${CSS.escape(id)}"]`);
  if (!j || !el) return renderQueue();
  const bar = el.querySelector(".bar > i");
  bar.style.width = j.pct + "%";
  bar.className = j.status === "done" ? "done" : "";
  const g = el.querySelector(".stat .grow");
  g.textContent = j.stat;
  g.title = j.stat;
  g.className = "grow" + (j.status === "error" ? " reason" : "");
}

function renderSummary() {
  const list = Array.from(jobs.values());
  const down = list.filter((j) => j.status === "downloading").length;
  const q = list.filter((j) => j.status === "queued").length;
  const done = list.filter((j) => j.status === "done").length;
  const err = list.filter((j) => j.status === "error").length;
  $("qsum").textContent = jobs.size === 0
    ? "Idle"
    : [`${down} downloading`, `${q} queued`, done && `${done} done`, err && `${err} failed`]
        .filter(Boolean).join(" · ");
}

queueEl.addEventListener("click", async (e) => {
  const cancel = e.target.closest("[data-cancel]");
  if (cancel) {
    const id = cancel.dataset.cancel;
    // Not started yet: it has no process, so just drop it from our own queue.
    if (pending.some((p) => p.id === id)) {
      pending = pending.filter((p) => p.id !== id);
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
  if (dismiss) { jobs.delete(dismiss.dataset.dismiss); renderQueue(); return; }

  const retry = e.target.closest("[data-retry]");
  if (retry) retryJob(retry.dataset.retry);
});

function retryJob(id) {
  const j = jobs.get(id);
  if (!j) return;
  j.status = "queued";
  j.pct = 0;
  j.stat = "Queued";
  j.reason = null;
  pending.push({ id, url: j.url, dir: j.dir, title: j.title, force: j.force || null });
  updateJobElement(id);
  renderQueue();
  pump();
}

$("clear").addEventListener("click", () => {
  for (const [id, j] of Array.from(jobs.entries())) {
    if (j.status === "done" || j.status === "error") jobs.delete(id);
  }
  renderQueue();
});
$("retryall").addEventListener("click", () => {
  Array.from(jobs.entries())
    .filter(([, j]) => j.status === "error")
    .forEach(([id]) => retryJob(id));
});
$("folder").addEventListener("click", () => { const ch = activeChannel(); if (ch) changeDir(ch); });
$("sync").addEventListener("click", () => {
  const ch = activeChannel();
  if (ch) doFetch(ch.url, { quiet: true });
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
  try { state = await invoke("load_state"); } catch { state = { active: null, channels: [] }; }
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
  showActive();
  renderQueue();
}
boot();