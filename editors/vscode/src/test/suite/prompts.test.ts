// The direct target's prompts match bragi-core's (these mirror crates/bragi-core/src/code.rs's tests).

import * as assert from 'assert';
import { cleanSuggestion, extractCode, rewritePrompt, rewriteResult, suggestPrompt } from '../../prompts';
import { EditorContext } from '../../types';

const ctx = (before: string, selection: string, after: string): EditorContext => ({ path: 'src/lib.rs', language: 'rust', before, selection, after, open_files: [], diagnostics: [] });

suite('prompts (direct target)', () => {
  test('code is read out of fences and thinking', () => {
    assert.strictEqual(extractCode('Here:\n```rust\nfn a() {}\n```\nDone.'), 'fn a() {}\n');
    assert.strictEqual(extractCode('````md\n```x```\n````'), '```x```\n');
    assert.strictEqual(extractCode('<think>hmm ```no```</think>\nlet a = 1;'), 'let a = 1;');
    assert.strictEqual(extractCode('```\ncut off'), 'cut off');
    assert.strictEqual(extractCode('\nplain\n'), 'plain');
  });

  test('rewrites keep the original trailing newline', () => {
    assert.strictEqual(rewriteResult('```\nx\n\n```', 'y\n'), 'x\n');
    assert.strictEqual(rewriteResult('```\nx\n```', 'y'), 'x');
  });

  test('suggestions drop repeats of the surrounding code', () => {
    const c = ctx('    let total = ', '', ';\n}\n');
    assert.strictEqual(cleanSuggestion('    let total = a + b', c), 'a + b');
    assert.strictEqual(cleanSuggestion('a + b;\n}', c), 'a + b');
    assert.strictEqual(cleanSuggestion('```rust\na + b\n```', c), 'a + b');
    assert.strictEqual(cleanSuggestion('   \n', c), '');
  });

  test('prompts mark the cursor and the selection like the core does', () => {
    const p = suggestPrompt(ctx('fn add(a: i32, b: i32) -> i32 {\n    ', '', '\n}\n'));
    assert.ok(p[1].content.includes('<code_before_cursor>\nfn add(a: i32, b: i32) -> i32 {\n    </code_before_cursor>'));
    const r = rewritePrompt(ctx('a\n', 'b\n', 'c\n'), 'make it loud', 'selection');
    assert.ok(r[1].content.includes('a\n<selection>b\n</selection>c\n'));
    assert.ok(rewritePrompt(ctx('a\n', '', 'c\n'), 'x', 'selection')[1].content.includes('the whole file'));
    assert.throws(() => rewritePrompt(ctx('a', '', ''), ' ', 'file'));
  });
});
