// What the grid shows: which videos pass the filters, in what order, and what
// each one's archive state is. No DOM and no Tauri, so the rules run under
// `node --test` (library.test.js).

export const isMembers = (v) => v.availability === "subscriber_only";

// "none"    not in this channel's archive
// "have"    archived and the file is on disk
// "missing" archived, but the file is gone
//
// `archived` maps id -> archive entry for the channel on screen. A download
// that just finished is `pending` until the archive is read again: it counts
// as had, though its path isn't known yet.
export function archiveState(id, archived) {
  const e = archived.get(id);
  if (!e) return "none";
  return e.path || e.pending ? "have" : "missing";
}

// Two questions, each with one answer at a time. A missing file still counts
// as archived: the record is there, and the card is where it says the file
// isn't.
const accessOk = (v, access) => access === "all" || (access === "members") === isMembers(v);
const haveOk = (v, have, archived) =>
  have === "all" || (have === "archived") === (archiveState(v.id, archived) !== "none");

export function sortVideos(list, sort) {
  const out = list.slice();
  switch (sort) {
    case "oldest": out.reverse(); break;
    case "popular": out.sort((a, b) => (b.view_count ?? -1) - (a.view_count ?? -1)); break;
    // Videos with no duration (shorts carry none in flat mode) go last either way.
    case "shortest": out.sort((a, b) => (a.duration ?? Infinity) - (b.duration ?? Infinity)); break;
    case "longest": out.sort((a, b) => (b.duration ?? -1) - (a.duration ?? -1)); break;
    // "newest" is the order YouTube returns. No upload date is available in
    // flat mode, so it can't be sorted on.
  }
  return out;
}

// The filtered, sorted list, plus the number behind every filter option. Each
// number is what choosing that option would show with the other filter left
// as it is, so the two selected options always carry the same number: the
// length of the list on screen.
export function view(list, { access, have, sort }, archived) {
  const count = (a, h) => list.filter((v) => accessOk(v, a) && haveOk(v, h, archived)).length;
  return {
    shown: sortVideos(list.filter((v) => accessOk(v, access) && haveOk(v, have, archived)), sort),
    counts: {
      access: { all: count("all", have), public: count("public", have), members: count("members", have) },
      have: { all: count(access, "all"), archived: count(access, "archived"), not: count(access, "not") },
    },
    hasMembers: list.some(isMembers),
  };
}

// What "Download all N" would queue: everything the filters leave that isn't on
// disk, isn't already in the queue, and can be downloaded at all. A missing
// file is fetched again like any other. `locked` counts the members-only videos
// left out for want of cookies.
export function downloadable(shown, archived, { cookies, holds }) {
  const jobs = [];
  let locked = 0;
  for (const video of shown) {
    const state = archiveState(video.id, archived);
    if (state === "have" || holds(video.id)) continue;
    if (isMembers(video) && !cookies) { locked++; continue; }
    jobs.push({ video });
  }
  return { jobs, locked };
}

// The batch sizes worth offering for `count` downloadable videos: only those
// smaller than the count, since "First 50" of 30 is just all of them.
export const BATCHES = [10, 25, 50, 100];
export const batchSizes = (count) => BATCHES.filter((n) => n < count);

// How many the Download button queues: the size picked from its menu, or
// everything when none is picked or fewer than that are left.
export const batchOf = (picked, count) => (picked && picked < count ? picked : count);

// Queueing more than this asks first: one click shouldn't start a few
// thousand downloads on a large channel.
export const ASK_ABOVE = 100;

// What a subscription downloads: the videos listed above the newest archived
// one. The listing is newest first, so that is exactly "uploaded since the
// last one I kept", with no dates needed. A video skipped on purpose stays
// skipped once something newer is archived.
//
// With nothing archived there is no such line, so only videos that weren't in
// the list before this refresh count (`before`: the ids it had, or null when
// there was no earlier list). Subscribing never pulls the back catalogue.
export function newVideos(list, archived, before) {
  const newest = list.findIndex((v) => archived.has(v.id));
  if (newest >= 0) return list.slice(0, newest);
  return before ? list.filter((v) => !before.has(v.id)) : [];
}

// "3 weeks ago" from a Unix timestamp. The listing's dates are approximate to
// begin with, so one unit is all the precision there is.
export function ago(timestamp, now = Date.now() / 1000) {
  if (!timestamp) return "";
  const s = Math.max(0, now - timestamp);
  const units = [["year", 31536000], ["month", 2592000], ["week", 604800], ["day", 86400], ["hour", 3600]];
  for (const [name, len] of units) {
    const n = Math.floor(s / len);
    if (n >= 1) return `${n} ${name}${n > 1 ? "s" : ""} ago`;
  }
  return "just now";
}

// Archived videos the channel no longer lists (deleted or made private on
// YouTube) have no card of their own, and they are the ones an archive most
// needs to keep showing.
//
// The list is in upload order, newest first, and that order is what Newest
// and Oldest sort by. An unlisted video has no place in it, so one with a
// recorded upload date goes in by date, ahead of the first listed video that
// is older; without a date it goes after the listed ones.
export function withUnlisted(list, entries, knownIds) {
  const extra = entries
    .filter((e) => !knownIds.has(e.id))
    .map((e) => ({
      id: e.id, title: e.title, duration: e.duration, view_count: e.view_count, url: e.url,
      availability: null, unlisted: true, timestamp: e.timestamp ?? null,
    }));
  if (!extra.length) return list;
  // A listed video's date: the exact one when it's archived, else the
  // listing's approximate one.
  const exact = new Map(entries.filter((e) => e.timestamp).map((e) => [e.id, e.timestamp]));
  const out = list.slice();
  const dated = extra.filter((x) => x.timestamp).sort((a, b) => b.timestamp - a.timestamp);
  let from = 0;
  for (const x of dated) {
    let at = out.length;
    for (let i = from; i < out.length; i++) {
      const t = out[i].unlisted ? out[i].timestamp : exact.get(out[i].id) ?? out[i].timestamp;
      if (t && t < x.timestamp) { at = i; break; }
    }
    out.splice(at, 0, x);
    from = at + 1;
  }
  return out.concat(extra.filter((x) => !x.timestamp));
}

// Which cards of a long grid to have on the page: the rows in view, plus
// `over` pixels of rows above and below so scrolling rarely catches a row not
// yet drawn. `top` is how far the view has scrolled past the grid's own top
// (negative while the grid starts below the view's edge), and `rowH` is a
// row's height with the gap under it. `before` and `after` are the heights
// the undrawn rows would have taken, so the page stays its full length and
// the scrollbar tells the truth.
export function windowOf({ count, cols, rowH, top, viewH, over }) {
  const rows = Math.ceil(count / cols);
  const clamp = (n, lo) => Math.min(rows, Math.max(lo, n));
  const firstRow = clamp(Math.floor((top - over) / rowH), 0);
  const lastRow = clamp(Math.ceil((top + viewH + over) / rowH), firstRow);
  return {
    first: firstRow * cols,
    last: Math.min(count, lastRow * cols),
    before: firstRow * rowH,
    after: (rows - lastRow) * rowH,
  };
}
