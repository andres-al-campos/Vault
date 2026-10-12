import { test } from "node:test";
import assert from "node:assert/strict";
import { createTabs, resumeAt, progress } from "./player.js";

const vid = (id) => ({ id, path: `/d/${id}.mp4`, title: id });

test("opening a video puts it on screen; opening it again makes no second tab", () => {
  const t = createTabs();
  t.open(vid("a"));
  t.open(vid("b"));
  t.open(vid("a"));
  assert.deepEqual(t.tabs.map((x) => x.id), ["a", "b"]);
  assert.equal(t.activeId, "a");
});

test("a video opened in the background leaves the one on screen alone", () => {
  const t = createTabs();
  t.open(vid("a"));
  t.open(vid("b"), { background: true });
  assert.equal(t.activeId, "a");
  assert.deepEqual(t.tabs.map((x) => x.id), ["a", "b"]);
});

test("the first video opened in the background still becomes the one on screen", () => {
  const t = createTabs();
  t.open(vid("a"), { background: true });
  assert.equal(t.activeId, "a");
});

test("closing the tab on screen moves right, or left from the last one", () => {
  const t = createTabs();
  ["a", "b", "c"].forEach((id) => t.open(vid(id)));
  t.select("b");
  t.close("b");
  assert.equal(t.activeId, "c");
  t.close("c");
  assert.equal(t.activeId, "a");
  t.close("a");
  assert.equal(t.activeId, null);
});

test("closing another tab keeps the one on screen", () => {
  const t = createTabs();
  ["a", "b"].forEach((id) => t.open(vid(id)));
  t.close("a");
  assert.equal(t.activeId, "b");
});

test("each tab keeps its own place, and reopening the video keeps it too", () => {
  const t = createTabs();
  t.open(vid("a"));
  t.note("a", 42, 300);
  t.open(vid("b"));
  t.note("b", 7, 60);
  assert.equal(resumeAt(t.find("a")), 42);
  t.open(vid("a"));
  assert.equal(resumeAt(t.active), 42);
});

test("a video left in its last seconds starts over", () => {
  assert.equal(resumeAt({ position: 297, duration: 300 }), 0);
  assert.equal(resumeAt({ position: 200, duration: 300 }), 200);
  assert.equal(resumeAt({ position: 12, duration: 0 }), 12, "length unknown: resume");
});

test("a position report without a length keeps the length already known", () => {
  const t = createTabs();
  t.open(vid("a"));
  t.note("a", 10, 300);
  t.note("a", 11, 0);
  assert.equal(t.find("a").duration, 300);
  assert.equal(progress(t.find("a")), 11 / 300);
});

test("open tabs and their places survive being saved and read back", () => {
  const t = createTabs();
  ["a", "b"].forEach((id) => t.open(vid(id)));
  t.note("a", 90, 600);
  const back = createTabs(JSON.parse(JSON.stringify(t.snapshot())));
  assert.equal(back.activeId, "b");
  assert.equal(resumeAt(back.find("a")), 90);
});

test("damaged saved tabs are dropped instead of breaking the player", () => {
  const back = createTabs({ activeId: "gone", tabs: [null, { id: 5 }, { id: "a", path: "/d/a.mp4", position: "x" }] });
  assert.deepEqual(back.tabs.map((x) => x.id), ["a"]);
  assert.equal(back.activeId, "a");
  assert.equal(back.find("a").position, 0);
  assert.equal(createTabs("nonsense").tabs.length, 0);
});
