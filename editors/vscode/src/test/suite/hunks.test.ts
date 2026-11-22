// The direct target's hunks match bragi-core's `code::FileDiff` (the same cases as
// crates/bragi-serve/tests/api.rs's `hunks_are_reviewed_and_applied_one_at_a_time`).

import * as assert from 'assert';
import { anchorLine, applyHunks, computeHunks, mapOffset, minimalEdit } from '../../hunks';

const twelve = Array.from({ length: 12 }, (_, i) => `line ${i + 1}\n`).join('');
const twoChanges = twelve.replace('line 2\n', 'LINE 2\n').replace('line 11\n', 'line 11\nadded\n');

suite('hunks (direct target)', () => {
  test('changes far apart are separate hunks, shaped like the core', () => {
    const h = computeHunks(twelve, twoChanges);
    assert.deepStrictEqual(
      h.hunks.map((x) => [x.old_start, x.old_len, x.new_start, x.new_len]),
      [
        [0, 5, 0, 5],
        [8, 4, 8, 5],
      ],
    );
    assert.deepStrictEqual([h.added, h.removed], [2, 1]);
    assert.deepStrictEqual(
      h.hunks[0].lines.map((l) => [l.tag, l.text, l.old_no, l.new_no]),
      [
        ['context', 'line 1', 1, 1],
        ['removed', 'line 2', 2, null],
        ['added', 'LINE 2', null, 2],
        ['context', 'line 3', 3, 3],
        ['context', 'line 4', 4, 4],
        ['context', 'line 5', 5, 5],
      ],
    );
    assert.deepStrictEqual(h.hunks.map(anchorLine), [1, 11]);
  });

  test('applying keeps only the accepted hunks', () => {
    assert.strictEqual(applyHunks(twelve, twoChanges, [true, true]).file, twoChanges);
    assert.strictEqual(applyHunks(twelve, twoChanges, []).file, twelve);
    const second = applyHunks(twelve, twoChanges, [false, true]);
    assert.strictEqual(second.file, twelve.replace('line 11\n', 'line 11\nadded\n'));
    assert.deepStrictEqual(second.remaining.hunks.map((x) => x.old_start), [0]);
  });

  test('line endings and a missing last newline are kept', () => {
    const a = 'a\r\nb\r\nc';
    const b = 'a\r\nB\r\nc';
    const h = computeHunks(a, b);
    assert.strictEqual(h.hunks[0].lines[1].text, 'b');
    assert.strictEqual(h.hunks[0].lines[3].newline, false);
    assert.strictEqual(applyHunks(a, b, [true]).file, b);
    assert.deepStrictEqual(computeHunks('', 'x\n').hunks.map((x) => [x.old_start, x.old_len, x.new_len]), [[0, 0, 1]]);
  });

  test('offsets move through unchanged lines, not changed ones', () => {
    const h = computeHunks(twelve, twoChanges).hunks;
    const at = (line: number, col = 0) => twelve.split('\n').slice(0, line).join('\n').length + (line ? 1 : 0) + col;
    const inNew = (line: number, col = 0) => twoChanges.split('\n').slice(0, line).join('\n').length + (line ? 1 : 0) + col;
    assert.strictEqual(mapOffset(twelve, twoChanges, h, at(5, 2)), inNew(5, 2));
    assert.strictEqual(mapOffset(twelve, twoChanges, h, at(11, 1)), inNew(12, 1), 'after the added line');
    assert.strictEqual(mapOffset(twelve, twoChanges, h, at(1, 1)), undefined, 'inside the removed line');
    assert.strictEqual(mapOffset(twelve, twoChanges, h, twelve.length), twoChanges.length);
  });

  test('the edit that applies a hunk is as small as it can be', () => {
    assert.deepStrictEqual(minimalEdit('abcdef', 'abXYef'), { start: 2, end: 4, text: 'XY' });
    assert.deepStrictEqual(minimalEdit('same', 'same'), { start: 4, end: 4, text: '' });
  });
});
