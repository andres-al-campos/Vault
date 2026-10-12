// The open videos: which ones are open, which one is on screen, and where each
// was left. No DOM and no Tauri in here, so the rules run under `node --test`
// (player.test.js).

// A video left this close to its end starts over instead of resuming into its
// last seconds.
export const FINISHED_WITHIN = 5;

const usable = (t) => t && typeof t.id === "string" && typeof t.path === "string";

export function createTabs(saved) {
  // What was saved comes back from storage, so it's checked before it's used.
  const tabs = (Array.isArray(saved?.tabs) ? saved.tabs : []).filter(usable).map((t) => ({
    id: t.id, path: t.path, title: String(t.title ?? t.id),
    position: Number(t.position) || 0, duration: Number(t.duration) || 0,
    still: typeof t.still === "string" ? t.still : null,
  }));
  let activeId = tabs.some((t) => t.id === saved?.activeId) ? saved.activeId : tabs[0]?.id ?? null;

  const find = (id) => tabs.find((t) => t.id === id) || null;

  // Opens a video in a tab of its own, or finds the tab it already has. In the
  // background it leaves the video on screen alone.
  function open({ id, path, title }, { background = false } = {}) {
    let tab = find(id);
    if (!tab) {
      tab = { id, path, title, position: 0, duration: 0, still: null };
      tabs.push(tab);
    } else {
      // The file may have moved since the tab was opened.
      tab.path = path;
    }
    if (!background || activeId === null) activeId = id;
    return tab;
  }

  // Closing the tab on screen moves to the one on its right, or its left when
  // it was the last.
  function close(id) {
    const at = tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    tabs.splice(at, 1);
    if (activeId === id) activeId = (tabs[at] ?? tabs[at - 1])?.id ?? null;
  }

  function select(id) {
    if (find(id)) activeId = id;
  }

  // Records where a video is. A duration of 0 means the player doesn't know
  // yet, and what was known is kept.
  function note(id, position, duration) {
    const tab = find(id);
    if (!tab) return;
    tab.position = Math.max(0, position);
    if (duration > 0) tab.duration = duration;
  }

  // The picture a tab was left on: a file holding that frame, shown while the
  // video loads again so a switch goes picture to picture.
  function keep(id, still) {
    const tab = find(id);
    if (tab && still) tab.still = still;
  }

  return {
    tabs, find, open, close, select, note, keep,
    get activeId() { return activeId; },
    get active() { return find(activeId); },
    snapshot: () => ({ activeId, tabs: tabs.map((t) => ({ ...t })) }),
  };
}

// Where a tab starts playing from.
export function resumeAt(tab) {
  if (!tab || !(tab.position > 0)) return 0;
  if (tab.duration > 0 && tab.position >= tab.duration - FINISHED_WITHIN) return 0;
  return tab.position;
}

// How far through a tab is, 0 to 1, for the line under its title.
export function progress(tab) {
  return tab?.duration > 0 ? Math.min(1, Math.max(0, tab.position / tab.duration)) : 0;
}
