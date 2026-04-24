import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import {
  buildActiveFileQuery,
  buildArtefactOverviewBatchQuery,
  buildSearchQuery,
} from '../../queryBuilder';

suite('queryBuilder', () => {
  test('buildActiveFileQuery uses overview and artefact fields', () => {
    const query = buildActiveFileQuery('src/main.ts', 25);

    assert.match(query, /selectArtefacts\(by: \{ path: "src\/main\.ts" \}\)/);
    assert.match(query, /overview/);
    assert.match(query, /artefacts\(first: 25\)/);
    assert.match(query, /symbolFqn/);
    assert.match(query, /canonicalKind/);
    assert.match(query, /summary/);
  });

  test('buildArtefactOverviewBatchQuery uses symbol selectors when available', () => {
    const batch = buildArtefactOverviewBatchQuery([
      {
        path: 'src/main.ts',
        symbolFqn: 'src/main.ts::main',
      },
    ]);

    assert.ok(batch);
    assert.match(batch.query, /artefact0: selectArtefacts\(by: \{\s*symbolFqn: "src\/main\.ts::main"/);
    assert.deepEqual(batch.aliases.map((entry) => entry.alias), ['artefact0']);
  });

  test('buildArtefactOverviewBatchQuery falls back to path and lines', () => {
    const batch = buildArtefactOverviewBatchQuery([
      {
        path: 'src/main.ts',
        lines: {
          start: 11,
          end: 19,
        },
      },
    ]);

    assert.ok(batch);
    assert.match(batch.query, /path: "src\/main\.ts"/);
    assert.match(batch.query, /lines: \{ start: 11, end: 19 \}/);
  });

  test('buildSearchQuery trims text and caps the requested fields', () => {
    const query = buildSearchQuery('  http handler  ', 10);

    assert.match(query, /search: "http handler"/);
    assert.match(query, /artefacts\(first: 10\)/);
    assert.match(query, /startLine/);
    assert.match(query, /endLine/);
    assert.match(query, /summary/);
  });
});
