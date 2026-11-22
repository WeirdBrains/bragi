// The direct target's copy of bragi-core's `code` module (crates/bragi-core/src/code.rs): the
// prompts and reading code out of answers. The Bragi target never uses this; `bragi serve` runs
// the Rust original. Keep the two in step (test/suite/prompts.test.ts mirrors the Rust tests).

import { EditorContext, Scope } from './types';

export interface PromptMessage {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

const SUGGEST_BEFORE_CHARS = 6_000;
const SUGGEST_AFTER_CHARS = 2_000;
const MAX_FILE_CHARS = 48_000;
const MAX_OPEN_FILE_CHARS = 8_000;
const MAX_OPEN_FILES = 3;
const MAX_DIAGNOSTICS = 40;

function tail(s: string, n: number): string {
  const chars = Array.from(s);
  if (chars.length <= n) {
    return s;
  }
  const cut = chars.slice(chars.length - n).join('');
  const i = cut.indexOf('\n');
  return i >= 0 && i < 200 ? cut.slice(i + 1) : cut;
}

function head(s: string, n: number): string {
  const chars = Array.from(s);
  if (chars.length <= n) {
    return s;
  }
  const cut = chars.slice(0, n).join('');
  const i = cut.lastIndexOf('\n');
  return i >= 0 && cut.length - i < 200 ? cut.slice(0, i + 1) : cut;
}

function fenceFor(text: string): string {
  const longest = Math.max(0, ...(text.match(/`+/g) ?? []).map((r) => r.length));
  return '`'.repeat(Math.max(longest, 2) + 1);
}

function fenced(lang: string, text: string): string {
  const f = fenceFor(text);
  return `${f}${lang}\n${text}${text.endsWith('\n') || text === '' ? '' : '\n'}${f}`;
}

function name(ctx: EditorContext): string {
  return ctx.path || 'untitled';
}

function diagnosticsText(ctx: EditorContext): string {
  return ctx.diagnostics
    .slice(0, MAX_DIAGNOSTICS)
    .map((d) => `line ${d.line}: ${d.severity || 'problem'}${d.source ? ` [${d.source}]` : ''}: ${d.message.trim()}`)
    .join('\n');
}

function openFilesText(ctx: EditorContext): string {
  return ctx.open_files
    .filter((f) => f.path !== ctx.path && f.text)
    .slice(0, MAX_OPEN_FILES)
    .map((f) => `Open file ${f.path}:\n${fenced(f.language, head(f.text, MAX_OPEN_FILE_CHARS))}\n\n`)
    .join('');
}

const SUGGEST_SYSTEM =
  'You are a code completion engine. You get the code before and after the cursor and answer ' +
  'with only the text to insert at the cursor: no explanation, no repetition of the surrounding code, no Markdown fences. ' +
  'Keep it short (one statement or block) and match the file\'s style and indentation. Answer with nothing if nothing fits.';

export function effectiveScope(ctx: EditorContext, scope: Scope): Scope {
  return scope === 'selection' && ctx.selection === '' ? 'file' : scope;
}

export function suggestPrompt(ctx: EditorContext): PromptMessage[] {
  const before = tail(ctx.before + ctx.selection, SUGGEST_BEFORE_CHARS);
  const after = head(ctx.after, SUGGEST_AFTER_CHARS);
  let user = `File: ${name(ctx)} (${ctx.language || 'plain text'})\n`;
  const diags = diagnosticsText(ctx);
  if (diags) {
    user += `Problems in this file:\n${diags}\n`;
  }
  user += `\n<code_before_cursor>\n${before}</code_before_cursor>\n<code_after_cursor>\n${after}</code_after_cursor>\n`;
  user += '\nWrite only the text that goes at the cursor.';
  return [
    { role: 'system', content: SUGGEST_SYSTEM },
    { role: 'user', content: user },
  ];
}

const REWRITE_SYSTEM =
  'You rewrite code as asked. Answer with the complete replacement in one fenced code block ' +
  'and nothing else: keep everything you were not asked to change, keep the style and indentation, and never leave ' +
  'placeholders such as "rest unchanged".';

export function rewritePrompt(ctx: EditorContext, instruction: string, scope: Scope): PromptMessage[] {
  instruction = instruction.trim();
  if (!instruction) {
    throw new Error('say how to rewrite the code');
  }
  scope = effectiveScope(ctx, scope);
  const file = ctx.before + ctx.selection + ctx.after;
  if (scope === 'file' && Array.from(file).length > MAX_FILE_CHARS) {
    throw new Error(`the file is over ${MAX_FILE_CHARS} characters; select the part to rewrite`);
  }
  let user = openFilesText(ctx);
  if (scope === 'file') {
    user += `The file ${name(ctx)}:\n${fenced(ctx.language, file)}\n\n`;
  } else {
    const marked = `${tail(ctx.before, MAX_FILE_CHARS / 2)}<selection>${ctx.selection}</selection>${head(ctx.after, MAX_FILE_CHARS / 4)}`;
    user += `The file ${name(ctx)}, with the part to rewrite between <selection> and </selection>:\n${fenced(ctx.language, marked)}\n\n`;
  }
  const diags = diagnosticsText(ctx);
  if (diags) {
    user += `Problems the editor reports in this file:\n${diags}\n\n`;
  }
  const what = scope === 'file' ? 'the whole file' : 'only the selected part (without the <selection> tags)';
  user += `Rewrite ${what}: ${instruction}\nAnswer with the replacement in one fenced code block.`;
  return [
    { role: 'system', content: REWRITE_SYSTEM },
    { role: 'user', content: user },
  ];
}

/** Editor context for a direct chat, as one block ahead of the question. */
export function chatContextText(ctx: EditorContext): string {
  let out = openFilesText(ctx);
  out += `The file ${name(ctx)}:\n${fenced(ctx.language, head(ctx.before + ctx.selection + ctx.after, MAX_FILE_CHARS))}\n\n`;
  if (ctx.selection) {
    out += `Selected in it:\n${fenced(ctx.language, ctx.selection)}\n\n`;
  }
  const diags = diagnosticsText(ctx);
  if (diags) {
    out += `Problems in it:\n${diags}\n\n`;
  }
  return out;
}

function stripThinking(answer: string): string {
  const s = answer.indexOf('<think>');
  const e = answer.indexOf('</think>');
  if (e >= 0 && (s < 0 || (s < e && answer.slice(0, s).trim() === ''))) {
    return answer.slice(e + '</think>'.length);
  }
  return answer;
}

export function extractCode(answer: string): string {
  answer = stripThinking(answer);
  const lines = answer.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trimStart();
    const ticks = /^`*/.exec(t)![0].length;
    if (ticks < 3) {
      continue;
    }
    const fence = t.slice(0, ticks);
    let body = '';
    for (const l of lines.slice(i + 1)) {
      const tt = l.trim();
      if (tt.startsWith(fence) && tt.replace(/^`+/, '') === '') {
        return body;
      }
      body += l;
    }
    return body;
  }
  return answer.replace(/^\n+|\n+$/g, '');
}

export function rewriteResult(answer: string, original: string): string {
  let code = extractCode(answer).replace(/\n+$/, '');
  if (original.endsWith('\n')) {
    code += '\n';
  }
  return code;
}

export function cleanSuggestion(answer: string, ctx: EditorContext): string {
  let s = extractCode(answer);
  const before = ctx.before + ctx.selection;
  const line = before.slice(before.lastIndexOf('\n') + 1);
  if (line.trim() !== '' && s.startsWith(line)) {
    s = s.slice(line.length);
  }
  const after = ctx.after.trimStart();
  if (after) {
    const first = (after.split('\n')[0] ?? '').trim();
    const i = first ? s.lastIndexOf(first) : -1;
    if (i >= 0) {
      const rest = s.slice(i).trimEnd();
      if (rest.length <= after.length && after.startsWith(rest)) {
        s = s.slice(0, i);
      }
    }
  }
  s = s.trimEnd();
  return s.trim() === '' ? '' : s;
}
