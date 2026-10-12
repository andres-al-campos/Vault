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

// What "Download N shown" would queue: everything on screen that isn't on
// disk, isn't already in the queue, and can be downloaded at all. A missing
// file is fetched again in force mode. `locked` counts the members-only videos
// left out for want of cookies.
export function downloadable(shown, archived, { cookies, holds }) {
  const jobs = [];
  let locked = 0;
  for (const video of shown) {
    const state = archiveState(video.id, archived);
    if (state === "have" || holds(video.id)) continue;
    if (isMembers(video) && !cookies) { locked++; continue; }
    jobs.push({ video, force: state === "missing" });
  }
  return { jobs, locked };
}

// Archived videos the channel no longer lists (deleted or made private on
// YouTube) have no card of their own, and they are the ones an archive most
// needs to keep showing. They go after the listed videos.
export function withUnlisted(list, entries, knownIds) {
  const extra = entries
    .filter((e) => !knownIds.has(e.id))
    .map((e) => ({
      id: e.id, title: e.title, duration: e.duration, view_count: e.view_count, url: e.url,
      availability: null, unlisted: true,
    }));
  return extra.length ? list.concat(extra) : list;
}
