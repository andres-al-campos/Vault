import { test } from "node:test";
import assert from "node:assert/strict";
import { createQueue } from "./queue.js";

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
