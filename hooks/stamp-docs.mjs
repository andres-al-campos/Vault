// Pre-commit: stamps the docs this commit changes, so nobody writes a stamp by hand.
//
// - A doc that describes the code (features/*.md, README.md, docs/*.md) gets "Updated <time>,
//   at <version>." under its title when its description changed. A change only to its Calls
//   leaves the stamp alone: adding a ruling doesn't mean anyone read the description again.
// - Each Principle (AGENTS.md) and Call (a feature file's ## Calls) that is new or reworded
//   starts "<time>, <version>. ". Text that already exists word for word somewhere in HEAD
//   keeps its time, so a Call moved between files keeps when it was made. A trailing
//   "Applies *Name*." is ignored when comparing, so citing a principle doesn't restamp.
//
// The time is the commit's; the version is Vault's (package.json). The stamp is a clue to how
// far to trust a doc, not a claim that anyone reread it. Turned on by `./build.sh`, which sets
// core.hooksPath to hooks/.

import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const git = (...args) => execFileSync('git', args, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'ignore'] });
const tryGit = (...args) => { try { return git(...args); } catch { return null; } };
const root = git('rev-parse', '--show-toplevel').trim();
process.chdir(root);

const version = JSON.parse(fs.readFileSync('package.json', 'utf8')).version;
const p = (n) => String(n).padStart(2, '0');
const d = new Date();
const now = `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;

const STAMP = /^(?:Checked|Updated) .+, at \S+\.$/;
const PREFIX = /^\d{4}-\d\d-\d\d \d\d:\d\d, \S+?\. /;
const describes = (f) => /^(features|docs)\/[^/]+\.md$/.test(f) || f === 'README.md';
const hasEntries = (f) => f === 'AGENTS.md' || /^features\/[^/]+\.md$/.test(f);

// The Principles and Calls in a text: { start, end, body } per entry, body without its time
// or cited principle, whitespace collapsed.
function entries(text) {
  const lines = text.split('\n');
  const found = [];
  let inSection = false;
  for (let i = 0; i < lines.length; i++) {
    if (/^#{1,6} /.test(lines[i])) { inSection = /^## (Principles|Calls)\b/.test(lines[i]); continue; }
    if (!inSection || !lines[i].startsWith('- ')) continue;
    let end = i + 1;
    while (end < lines.length && /^\s+\S/.test(lines[end])) end++;
    const body = lines.slice(i, end).join(' ').slice(2).replace(PREFIX, '')
      .replace(/\s*Applies \*.*$/, '').replace(/\s+/g, ' ').trim();
    found.push({ start: i, end, body });
    i = end - 1;
  }
  return found;
}

// A doc's description: everything but its stamp and its Calls.
function description(text) {
  let inCalls = false;
  return text.split('\n').filter((line) => {
    if (/^#{1,6} /.test(line)) inCalls = /^## Calls\b/.test(line);
    return !inCalls && !STAMP.test(line);
  }).join('\n').replace(/\s+/g, ' ').trim();
}

// A first commit has no HEAD, so nothing is known yet.
const headDocs = (tryGit('ls-tree', '-r', '--name-only', 'HEAD') ?? '').split('\n').filter(hasEntries);
const known = new Set(headDocs.flatMap((f) => entries(tryGit('show', `HEAD:${f}`) ?? '').map((e) => e.body)));

function stamp(file, before, after) {
  const lines = after.split('\n');
  if (hasEntries(file)) {
    for (const e of entries(after)) {
      if (known.has(e.body)) continue;
      lines[e.start] = `- ${now}, ${version}. ${lines[e.start].slice(2).replace(PREFIX, '')}`;
    }
  }
  const at = lines.findIndex((l) => STAMP.test(l));
  if (describes(file) && (at < 0 || before === null || description(before) !== description(after))) {
    const line = `Updated ${now}, at ${version}.`;
    if (at >= 0) lines[at] = line;
    else {
      const title = lines.findIndex((l) => l.startsWith('# '));
      lines.splice(title + 1, 0, '', line);
    }
  }
  return lines.join('\n');
}

const staged = git('diff', '--cached', '--name-only', '--diff-filter=ACMR').split('\n')
  .filter((f) => describes(f) || hasEntries(f));
for (const file of staged) {
  const before = tryGit('show', `HEAD:${file}`);
  const index = git('show', `:${file}`);
  const stampedIndex = stamp(file, before, index);
  if (stampedIndex === index) continue;
  // Restamp the staged text and the working file separately, so unstaged edits stay unstaged.
  const blob = execFileSync('git', ['hash-object', '-w', '--stdin'], { input: stampedIndex, encoding: 'utf8' }).trim();
  git('update-index', '--cacheinfo', `100644,${blob},${file}`);
  const work = fs.readFileSync(file, 'utf8');
  fs.writeFileSync(file, work === index ? stampedIndex : stamp(file, before, work));
  console.log(`stamped ${file}`);
}
