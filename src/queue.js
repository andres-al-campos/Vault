// The download queue's state: which jobs exist, which are waiting, and which
// one is running. No DOM and no Tauri in here, so the rules run under
// `node --test` (queue.test.js).
//
// A job's facts live on the job and nowhere else.
//
// `saved` is an earlier snapshot(): the jobs that hadn't finished, and whether
// the queue was stopped.
export function createQueue(saved) {
  const jobs = new Map(); // id -> { title, url, dir, pct, status, stat }
  let waiting = [];       // ids, in the order they will start
  let activeId = null;    // the one job downloading (concurrency = 1)
  let stopped = false;    // nothing starts until resume()
  // The running job stop() asked to end. Its process takes a moment to exit,
  // and until it reports, the job still counts as running: resuming in that
  // moment must not start it a second time.
  let halting = null;

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
    if (stopped || activeId || waiting.length === 0) return null;
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
    const held = halting === id;
    if (held) halting = null;
    // Stopped, not failed: it goes back to the front of the line, and its
    // partial file is what the next run carries on from.
    if (held && !ok) {
      activeId = null;
      waiting.unshift(id);
      Object.assign(j, { status: "queued", stat: j.pct > 0 ? `Stopped at ${Math.round(j.pct)}%` : "Queued" });
      return true;
    }
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

  // A failed job isn't finished: it stays, with its reason and its Retry,
  // until it is retried or dismissed. Each kind is cleared on its own.
  const clear = (status) => () => {
    for (const [id, j] of Array.from(jobs)) {
      if (j.status === status) jobs.delete(id);
    }
  };
  const clearFinished = clear("done");
  const clearFailed = clear("error");

  // Holds the line. Returns the running job's id, for the caller to end its
  // process; that job rejoins the front when the process reports.
  function stop() {
    stopped = true;
    if (!activeId || halting) return null;
    halting = activeId;
    jobs.get(activeId).stat = "Stopping…";
    return activeId;
  }

  function resume() { stopped = false; }

  // Moves a waiting job to just ahead of another, or to the end with no
  // `beforeId`. The rows are listed in the map's order, so the waiting ones
  // are dealt back into the places waiting jobs hold.
  function move(id, beforeId = null) {
    if (!waiting.includes(id) || id === beforeId) return false;
    const rest = waiting.filter((w) => w !== id);
    const at = beforeId === null ? rest.length : rest.indexOf(beforeId);
    if (at < 0) return false;
    rest.splice(at, 0, id);
    waiting = rest;
    const entries = Array.from(jobs);
    const inLine = new Set(waiting);
    let k = 0;
    jobs.clear();
    for (const [i, j] of entries) {
      const take = inLine.has(i) ? waiting[k++] : i;
      jobs.set(take, take === i ? j : entries.find(([e]) => e === take)[1]);
    }
    return true;
  }

  // Drops every job that hasn't started. Returns their ids.
  function clearWaiting() {
    const gone = waiting;
    waiting = [];
    for (const id of gone) jobs.delete(id);
    return gone;
  }

  // What to bring back after a restart: the running job first, then the line.
  function snapshot() {
    const ids = activeId ? [activeId, ...waiting] : waiting;
    return { stopped, jobs: ids.map((id) => { const { title, url, dir } = jobs.get(id); return { id, title, url, dir }; }) };
  }

  for (const j of Array.isArray(saved?.jobs) ? saved.jobs : []) {
    if (j && typeof j.id === "string" && typeof j.url === "string") add(j);
  }
  stopped = !!saved?.stopped;

  return {
    jobs, holds, add, retry, next, settle, remove, clearFinished, clearFailed, stop, resume, move, clearWaiting, snapshot,
    isWaiting: (id) => waiting.includes(id),
    get stopped() { return stopped; },
  };
}
