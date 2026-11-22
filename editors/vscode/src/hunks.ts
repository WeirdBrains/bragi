// Hunks: the reviewable pieces of a rewrite, shaped like bragi-core's `code::Hunk` (what
// `bragi serve` returns from /v1/code/hunks and /v1/code/apply, the same review the terminal app
// does). The local versions here are only for the direct target, which has no Bragi behind it,
// and keep the same rules: line endings kept byte for byte, applying every hunk gives the new
// text and applying none gives the old.

export type LineTag = 'context' | 'removed' | 'added';

export interface HunkLine {
  tag: LineTag;
  /** The line without its line ending. */
  text: string;
  newline: boolean;
  /** 1-based line numbers in the old and new text. */
  old_no?: number | null;
  new_no?: number | null;
}

/** One change with its context. Starts are 0-based line indexes. */
export interface Hunk {
  old_start: number;
  old_len: number;
  new_start: number;
  new_len: number;
  lines: HunkLine[];
}

export interface HunkList {
  hunks: Hunk[];
  added: number;
  removed: number;
}

export interface Applied {
  /** The old text with only the accepted hunks. */
  file: string;
  /** What is left to review, from `file` to the new text. */
  remaining: HunkList;
}

/** Lines of context around each change, as `diff -u` (and bragi-core) uses. */
export const CONTEXT = 3;

/** `text` split into lines, each keeping its line ending. */
export function lines(text: string): string[] {
  const out: string[] = [];
  let at = 0;
  for (;;) {
    const nl = text.indexOf('\n', at);
    if (nl < 0) {
      if (at < text.length) {
        out.push(text.slice(at));
      }
      return out;
    }
    out.push(text.slice(at, nl + 1));
    at = nl + 1;
  }
}

type Op = { tag: LineTag; i: number; j: number };

/** A line diff: common ends trimmed, the middle by longest common subsequence. */
function ops(a: string[], b: string[]): Op[] {
  let pre = 0;
  while (pre < a.length && pre < b.length && a[pre] === b[pre]) {
    pre++;
  }
  let suf = 0;
  while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf]) {
    suf++;
  }
  const n = a.length - pre - suf;
  const m = b.length - pre - suf;
  const raw: LineTag[] = [];
  if (n > 0 && m > 0 && n * m <= 4_000_000) {
    const w = m + 1;
    const dp = new Uint32Array((n + 1) * w);
    for (let i = n - 1; i >= 0; i--) {
      for (let j = m - 1; j >= 0; j--) {
        dp[i * w + j] = a[pre + i] === b[pre + j] ? dp[(i + 1) * w + j + 1] + 1 : Math.max(dp[(i + 1) * w + j], dp[i * w + j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && a[pre + i] === b[pre + j]) {
        raw.push('context');
        i++;
        j++;
      } else if (j >= m || (i < n && dp[(i + 1) * w + j] >= dp[i * w + j + 1])) {
        raw.push('removed');
        i++;
      } else {
        raw.push('added');
        j++;
      }
    }
  } else {
    // Too big to compare line by line (or one side empty): one replacement.
    raw.push(...Array<LineTag>(n).fill('removed'), ...Array<LineTag>(m).fill('added'));
  }
  // Removals before additions in each run of changes, as bragi-core shows them.
  const tags: LineTag[] = [...Array<LineTag>(pre).fill('context')];
  for (let k = 0; k < raw.length; ) {
    if (raw[k] === 'context') {
      tags.push('context');
      k++;
      continue;
    }
    let del = 0;
    let ins = 0;
    while (k < raw.length && raw[k] !== 'context') {
      raw[k++] === 'removed' ? del++ : ins++;
    }
    tags.push(...Array<LineTag>(del).fill('removed'), ...Array<LineTag>(ins).fill('added'));
  }
  tags.push(...Array<LineTag>(suf).fill('context'));
  const out: Op[] = [];
  let i = 0;
  let j = 0;
  for (const tag of tags) {
    out.push({ tag, i, j });
    if (tag !== 'added') {
      i++;
    }
    if (tag !== 'removed') {
      j++;
    }
  }
  return out;
}

function hunkLine(tag: LineTag, raw: string, oldNo: number | null, newNo: number | null): HunkLine {
  const newline = raw.endsWith('\n');
  let text = newline ? raw.slice(0, -1) : raw;
  if (text.endsWith('\r')) {
    text = text.slice(0, -1);
  }
  return { tag, text, newline, old_no: oldNo, new_no: newNo };
}

/** The hunks from `oldText` to `newText`. */
export function computeHunks(oldText: string, newText: string, context = CONTEXT): HunkList {
  const a = lines(oldText);
  const b = lines(newText);
  const all = ops(a, b);
  const changed = all.map((o, k) => (o.tag === 'context' ? -1 : k)).filter((k) => k >= 0);
  const hunks: Hunk[] = [];
  let k = 0;
  while (k < changed.length) {
    let last = k;
    while (last + 1 < changed.length && changed[last + 1] - changed[last] - 1 <= 2 * context) {
      last++;
    }
    const from = Math.max(0, changed[k] - context);
    const to = Math.min(all.length, changed[last] + context + 1);
    const slice = all.slice(from, to);
    const h: Hunk = { old_start: slice[0].i, old_len: 0, new_start: slice[0].j, new_len: 0, lines: [] };
    for (const o of slice) {
      if (o.tag === 'context') {
        h.lines.push(hunkLine('context', a[o.i], o.i + 1, o.j + 1));
      } else if (o.tag === 'removed') {
        h.lines.push(hunkLine('removed', a[o.i], o.i + 1, null));
      } else {
        h.lines.push(hunkLine('added', b[o.j], null, o.j + 1));
      }
      h.old_len += o.tag === 'added' ? 0 : 1;
      h.new_len += o.tag === 'removed' ? 0 : 1;
    }
    hunks.push(h);
    k = last + 1;
  }
  return { hunks, ...countLines(hunks) };
}

export function countLines(hunks: Hunk[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const h of hunks) {
    for (const l of h.lines) {
      added += l.tag === 'added' ? 1 : 0;
      removed += l.tag === 'removed' ? 1 : 0;
    }
  }
  return { added, removed };
}

/** `oldText` with only the hunks marked `true` taken from `newText` (missing entries: rejected). */
export function applyHunks(oldText: string, newText: string, accepted: boolean[]): Applied {
  const a = lines(oldText);
  const b = lines(newText);
  let out = '';
  let at = 0;
  computeHunks(oldText, newText).hunks.forEach((h, i) => {
    out += a.slice(at, h.old_start).join('');
    out += (accepted[i] ? b.slice(h.new_start, h.new_start + h.new_len) : a.slice(h.old_start, h.old_start + h.old_len)).join('');
    at = h.old_start + h.old_len;
  });
  out += a.slice(Math.min(at, a.length)).join('');
  return { file: out, remaining: computeHunks(out, newText) };
}

/** The smallest single edit that turns `from` into `to`: replace `[start, end)` of `from` with `text`. */
export function minimalEdit(from: string, to: string): { start: number; end: number; text: string } {
  let start = 0;
  while (start < from.length && start < to.length && from[start] === to[start]) {
    start++;
  }
  let tail = 0;
  while (tail < from.length - start && tail < to.length - start && from[from.length - 1 - tail] === to[to.length - 1 - tail]) {
    tail++;
  }
  return { start, end: from.length - tail, text: to.slice(start, to.length - tail) };
}

/** The 0-based old line a hunk's first change is at (where its accept and reject go). */
export function anchorLine(h: Hunk): number {
  let line = h.old_start;
  for (const l of h.lines) {
    if (l.tag === 'removed') {
      return (l.old_no ?? line + 1) - 1;
    }
    if (l.tag === 'added') {
      return line;
    }
    line++;
  }
  return line;
}

/**
 * Where `offset` in `oldText` is in `newText`, through lines the hunks leave alone (the same
 * line, the same column); undefined inside a changed line.
 */
export function mapOffset(oldText: string, newText: string, hunks: Hunk[], offset: number): number | undefined {
  const a = lines(oldText);
  let line = 0;
  let start = 0;
  while (line < a.length && start + a[line].length <= offset) {
    start += a[line].length;
    line++;
  }
  const col = offset - start;
  let target: number | undefined;
  let delta = 0;
  for (const h of hunks) {
    if (line < h.old_start) {
      break;
    }
    if (line < h.old_start + h.old_len) {
      const same = h.lines.find((l) => l.tag === 'context' && l.old_no === line + 1);
      if (!same || same.new_no == null) {
        return undefined;
      }
      target = same.new_no - 1;
      break;
    }
    delta += h.new_len - h.old_len;
  }
  target ??= line + delta;
  const b = lines(newText);
  if (target > b.length) {
    return undefined;
  }
  let at = 0;
  for (let k = 0; k < target; k++) {
    at += b[k].length;
  }
  if (target === b.length) {
    return col === 0 ? at : undefined;
  }
  return col <= b[target].length ? at + col : undefined;
}

/** A hunk as `diff -u` text, for hovers. */
export function hunkText(h: Hunk): string {
  return h.lines.map((l) => (l.tag === 'context' ? ' ' : l.tag === 'removed' ? '-' : '+') + l.text).join('\n');
}
