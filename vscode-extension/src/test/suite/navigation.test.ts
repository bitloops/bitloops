import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import {
  canonicalKindIconId,
  canonicalKindLabel,
  formatSearchResultDescription,
  toZeroBasedLineRange,
} from '../../navigation';

suite('navigation', () => {
  test('toZeroBasedLineRange maps one-based line numbers to editor ranges', () => {
    assert.deepEqual(
      toZeroBasedLineRange({
        startLine: 10,
        endLine: 13,
      }),
      {
        startLine: 9,
        endLine: 12,
      },
    );
  });

  test('formatSearchResultDescription includes the kind label when present', () => {
    assert.equal(
      formatSearchResultDescription({
        path: 'src/main.ts',
        canonicalKind: 'FUNCTION',
        symbolFqn: 'src/main.ts::main',
        startLine: 3,
        endLine: 9,
      }),
      'src/main.ts:3-9 · function',
    );
  });

  test('canonical kind helpers map to labels and theme icon ids', () => {
    assert.equal(canonicalKindLabel('TYPE_PARAMETER'), 'type parameter');
    assert.equal(canonicalKindIconId('FUNCTION'), 'symbol-function');
    assert.equal(canonicalKindIconId(undefined), undefined);
  });
});
