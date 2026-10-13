import { test } from "node:test";
import assert from "node:assert/strict";
import { createQueue, lineAction } from "./queue.js";

const job = (id, extra = {}) => ({ id, title: id, url: `u/${id}`, dir: "/d", ...extra });

test("jobs start one at a time, in the order they were added", () => {
  const q = createQueue();
  q.add(job("a"));
  q.add(job("b"));
  assert.equal(q.next().id, "a");
  assert.equal(q.next(), null, "b must wait while a is running");
  q.settle("a", true);
  assert.equal(q.next().id, "b");
});

test("a queued, running or finished job can't be added twice", () => {
  const q = createQueue();
  assert.equal(q.add(job("a")), true);
  assert.equal(q.add(job("a")), false);
  q.next();
  assert.equal(q.add(job("a")), false);
  q.settle("a", true);
  assert.equal(q.add(job("a")), false);
});

test("a failed job can be retried, and keeps where it was going", () => {
  const q = createQueue();
  q.add(job("a", { dir: "/elsewhere" }));
  q.next();
  q.settle("a", false, "Failed — Network error");
  assert.equal(q.retry("a"), true);
  assert.equal(q.next().dir, "/elsewhere");
});

test("a failed job can be queued again from its card", () => {
  const q = createQueue();
  q.add(job("a"));
  q.next();
  q.settle("a", false);
  assert.equal(q.add(job("a")), true);
  assert.equal(q.next().id, "a");
});

test("a second report for a settled job changes nothing", () => {
  const q = createQueue();
  q.add(job("a"));
  q.next();
  assert.equal(q.settle("a", true, "Already in archive"), true);
  assert.equal(q.settle("a", true, "Complete"), false);
  assert.equal(q.jobs.get("a").stat, "Already in archive");
});

test("cancelling a waiting job takes it out of the line", () => {
  const q = createQueue();
  q.add(job("a"));
  q.add(job("b"));
  q.next();
  assert.equal(q.isWaiting("b"), true);
  q.settle("b", false, "Cancelled");
  assert.equal(q.isWaiting("b"), false);
  q.settle("a", true);
  assert.equal(q.next(), null);
});

test("retry only applies to failed jobs", () => {
  const q = createQueue();
  q.add(job("a"));
  assert.equal(q.retry("a"), false);
  assert.equal(q.retry("nope"), false);
});

test("a stopped queue starts nothing, and the running job rejoins the front", () => {
  const q = createQueue();
  q.add(job("a")); q.add(job("b"));
  q.next();
  q.jobs.get("a").pct = 42.4;
  assert.equal(q.stop(), "a");
  assert.equal(q.next(), null);
  // The process exits a moment later and reports a cancel.
  assert.equal(q.settle("a", false, "Cancelled"), true);
  assert.equal(q.jobs.get("a").status, "queued");
  assert.equal(q.jobs.get("a").stat, "Stopped at 42%");
  assert.equal(q.next(), null);
  q.resume();
  assert.equal(q.next().id, "a");
});

test("resuming before the stopped download has exited doesn't start it twice", () => {
  const q = createQueue();
  q.add(job("a")); q.add(job("b"));
  q.next();
  q.stop();
  q.resume();
  assert.equal(q.next(), null);
  q.settle("a", false, "Cancelled");
  assert.equal(q.next().id, "a");
});

test("a download that finishes while being stopped is done, not requeued", () => {
  const q = createQueue();
  q.add(job("a"));
  q.next();
  q.stop();
  q.settle("a", true);
  assert.equal(q.jobs.get("a").status, "done");
  q.resume();
  assert.equal(q.next(), null);
});

test("a waiting job can be moved ahead of another, or to the end", () => {
  const q = createQueue();
  for (const id of ["a", "b", "c", "d"]) q.add(job(id));
  q.next(); // a is running
  assert.equal(q.move("d", "b"), true);
  assert.deepEqual([...q.jobs.keys()], ["a", "d", "b", "c"]);
  assert.equal(q.move("d"), true);
  assert.deepEqual([...q.jobs.keys()], ["a", "b", "c", "d"]);
  assert.equal(q.move("a", "b"), false); // the running one stays put
  q.settle("a", true);
  assert.equal(q.next().id, "b");
});

test("clearing the queue drops what hasn't started and keeps the rest", () => {
  const q = createQueue();
  for (const id of ["a", "b", "c"]) q.add(job(id));
  q.next();
  assert.deepEqual(q.clearWaiting(), ["b", "c"]);
  assert.deepEqual([...q.jobs.keys()], ["a"]);
  assert.equal(q.holds("b"), false);
});

test("a saved queue comes back in order, running job first, still stopped", () => {
  const q = createQueue();
  for (const id of ["a", "b", "c"]) q.add(job(id));
  q.next();
  q.settle("a", true);
  q.next(); // b is running
  q.stop();
  const again = createQueue(JSON.parse(JSON.stringify(q.snapshot())));
  assert.deepEqual([...again.jobs.keys()], ["b", "c"]);
  assert.equal(again.jobs.get("b").dir, "/d");
  assert.equal(again.stopped, true);
  assert.equal(again.next(), null);
  assert.equal(createQueue({ jobs: "junk" }).jobs.size, 0);
});

test("finished and failed jobs are cleared separately", () => {
  const q = createQueue();
  for (const id of ["a", "b", "c"]) q.add(job(id));
  q.next(); q.settle("a", true);
  q.next(); q.settle("b", false, "Network error");
  q.clearFinished();
  assert.deepEqual([...q.jobs.keys()], ["b", "c"]);
  assert.equal(q.jobs.get("b").stat, "Network error");
  q.clearFailed();
  assert.deepEqual([...q.jobs.keys()], ["c"]);
});

test("the first button releases, holds, or retries, whichever gets things moving", () => {
  assert.equal(lineAction({ stopped: false, live: 3, failed: 1 }), "stop");
  assert.equal(lineAction({ stopped: true, live: 3, failed: 0 }), "resume");
  // Held with nothing in line: still offered, or new downloads would wait unexplained.
  assert.equal(lineAction({ stopped: true, live: 0, failed: 0 }), "resume");
  assert.equal(lineAction({ stopped: false, live: 0, failed: 2 }), "retry");
  assert.equal(lineAction({ stopped: false, live: 0, failed: 0 }), null);
});

test("a retried job keeps its row and its turn", () => {
  const q = createQueue();
  for (const id of ["a", "b", "c", "d"]) q.add(job(id));
  q.next(); q.settle("a", false, "Network error");
  q.next(); // b is running
  assert.equal(q.retry("a"), true);
  assert.deepEqual([...q.jobs.keys()], ["a", "b", "c", "d"]);
  q.settle("b", true);
  assert.equal(q.next().id, "a");
  q.settle("a", true);
  assert.equal(q.next().id, "c");
});
