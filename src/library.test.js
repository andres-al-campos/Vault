import { test } from "node:test";
import assert from "node:assert/strict";
import { archiveState, view, downloadable, withUnlisted, sortVideos, batchSizes } from "./library.js";

const v = (id, extra = {}) => ({ id, title: id, duration: 10, view_count: 1, url: `u/${id}`, availability: null, ...extra });
const member = (id, extra = {}) => v(id, { availability: "subscriber_only", ...extra });

// a: public, on disk     b: public, not archived   c: members, on disk
// d: members, not archived   e: public, archived but the file is gone
const list = [v("a"), v("b"), member("c"), member("d"), v("e")];
const archived = new Map([
  ["a", { id: "a", path: "/d/a.mp4" }],
  ["c", { id: "c", path: "/d/c.mp4" }],
  ["e", { id: "e", path: null, dir: "/d" }],
]);
const ids = (r) => r.shown.map((x) => x.id);
const see = (access, have) => view(list, { access, have, sort: "newest" }, archived);

test("a video is none, have or missing by its archive entry", () => {
  assert.equal(archiveState("b", archived), "none");
  assert.equal(archiveState("a", archived), "have");
  assert.equal(archiveState("e", archived), "missing");
  // Just downloaded, archive not re-read yet.
  assert.equal(archiveState("z", new Map([["z", { id: "z", path: null, pending: true }]])), "have");
});

test("the two filters narrow each other", () => {
  assert.deepEqual(ids(see("all", "all")), ["a", "b", "c", "d", "e"]);
  assert.deepEqual(ids(see("all", "archived")), ["a", "c", "e"]);
  assert.deepEqual(ids(see("all", "not")), ["b", "d"]);
  assert.deepEqual(ids(see("members", "archived")), ["c"]);
  assert.deepEqual(ids(see("members", "not")), ["d"]);
  assert.deepEqual(ids(see("public", "archived")), ["a", "e"]);
  assert.deepEqual(ids(see("public", "not")), ["b"]);
});

test("a missing file counts as archived", () => {
  assert.ok(ids(see("all", "archived")).includes("e"));
  assert.ok(!ids(see("all", "not")).includes("e"));
});

test("each option's number is what choosing it would show", () => {
  const r = see("members", "not");
  assert.deepEqual(r.counts.access, { all: 2, public: 1, members: 1 });
  assert.deepEqual(r.counts.have, { all: 2, archived: 1, not: 1 });
  // The selected options both carry the length of the list on screen.
  assert.equal(r.counts.access.members, r.shown.length);
  assert.equal(r.counts.have.not, r.shown.length);
});

test("hasMembers says whether the access filter has anything to offer", () => {
  assert.equal(see("all", "all").hasMembers, true);
  assert.equal(view([v("a")], { access: "all", have: "all", sort: "newest" }, archived).hasMembers, false);
});

test("Download shown takes what isn't on disk, forcing the missing ones", () => {
  const holds = () => false;
  const open = downloadable(list, archived, { cookies: true, holds });
  assert.deepEqual(open.jobs.map((j) => [j.video.id, j.force]), [["b", false], ["d", false], ["e", true]]);
  assert.equal(open.locked, 0);

  const noCookies = downloadable(list, archived, { cookies: false, holds });
  assert.deepEqual(noCookies.jobs.map((j) => j.video.id), ["b", "e"]);
  assert.equal(noCookies.locked, 1);

  const queued = downloadable(list, archived, { cookies: true, holds: (id) => id === "b" });
  assert.deepEqual(queued.jobs.map((j) => j.video.id), ["d", "e"]);
});

test("only batches smaller than what there is to download are offered", () => {
  assert.deepEqual(batchSizes(8), []);
  assert.deepEqual(batchSizes(10), []);
  assert.deepEqual(batchSizes(60), [10, 25, 50]);
  assert.deepEqual(batchSizes(4000), [10, 25, 50, 100]);
});

test("a batch is the first downloadable videos in the order shown", () => {
  // Oldest first: the batch is the oldest not yet on disk, skipping archived ones.
  const oldest = view(list, { access: "all", have: "all", sort: "oldest" }, archived).shown;
  const { jobs } = downloadable(oldest, archived, { cookies: true, holds: () => false });
  assert.deepEqual(jobs.slice(0, 2).map((j) => j.video.id), ["e", "d"]);
});

test("archived videos the channel no longer lists are added after the listed ones", () => {
  const entries = [{ id: "a", title: "a" }, { id: "gone", title: "Deleted upload", duration: 5, view_count: null, url: "u/gone" }];
  const out = withUnlisted(list, entries, new Set(list.map((x) => x.id)));
  assert.equal(out.length, list.length + 1);
  assert.deepEqual([out.at(-1).id, out.at(-1).title, out.at(-1).unlisted], ["gone", "Deleted upload", true]);
  // Nothing unlisted: the same list comes back.
  assert.equal(withUnlisted(list, [{ id: "a" }], new Set(["a"])), list);
});

test("sorts", () => {
  const l = [v("x", { duration: 30, view_count: 5 }), v("y", { duration: null, view_count: 9 }), v("z", { duration: 3, view_count: 1 })];
  const order = (s) => sortVideos(l, s).map((x) => x.id).join("");
  assert.equal(order("newest"), "xyz");
  assert.equal(order("oldest"), "zyx");
  assert.equal(order("popular"), "yxz");
  assert.equal(order("shortest"), "zxy");
  assert.equal(order("longest"), "xzy");
});
