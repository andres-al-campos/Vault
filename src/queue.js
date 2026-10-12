// The download queue's state: which jobs exist, which are waiting, and which
// one is running. No DOM and no Tauri in here, so the rules run under
// `node --test` (queue.test.js).
//
// A job's facts live on the job and nowhere else.
export function createQueue() {
  const jobs = new Map(); // id -> { title, url, dir, pct, status, stat }
  let waiting = [];       // ids, in the order they will start
  let activeId = null;    // the one job downloading (concurrency = 1)

  // A queued, running or finished job holds its id. A failed one doesn't, so
  // the same video can be queued again.
  const holds = (id) => jobs.has(id) && jobs.get(id).status !== "error";

  function add({ id, title, url, dir }) {
    if (holds(id)) return false;
    jobs.set(id, { title, url, dir, pct: 0, status: "queued", stat: "Queued" });
    waiting.push(id);
    return true;
  }

  function retry(id) {
    const j = jobs.get(id);
    return !!j && j.status === "error" && add({ id, ...j });
  }

  // Starts the next waiting job when nothing is running, and returns it.
  function next() {
    if (activeId || waiting.length === 0) return null;
    activeId = waiting.shift();
    const j = jobs.get(activeId);
    Object.assign(j, { status: "downloading", stat: "Starting…", pct: 0 });
    return { id: activeId, ...j };
  }

  // Ends a queued or running job. Returns false when there was nothing to end:
  // a skipped download reports twice (the skip line, then the process exit),
  // and the second report must not overwrite the first.
  function settle(id, ok, stat) {
    const j = jobs.get(id);
    if (!j || j.status === "done" || j.status === "error") return false;
    waiting = waiting.filter((w) => w !== id);
    if (activeId === id) activeId = null;
    j.status = ok ? "done" : "error";
    if (ok) j.pct = 100;
    j.stat = stat || (ok ? "Complete" : "Failed");
    return true;
  }

  function remove(id) {
    jobs.delete(id);
    waiting = waiting.filter((w) => w !== id);
  }

  function clearFinished() {
    for (const [id, j] of Array.from(jobs)) {
      if (j.status === "done" || j.status === "error") jobs.delete(id);
    }
  }

  return {
    jobs, holds, add, retry, next, settle, remove, clearFinished,
    isWaiting: (id) => waiting.includes(id),
  };
}
