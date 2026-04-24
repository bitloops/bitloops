import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import {
  buildActiveFileQuery,
  buildArtefactOverviewBatchQuery,
  buildSelectionDetailsQuery,
  buildSearchQuery,
  buildStageItemsQuery,
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
    assert.match(query, /embeddingRepresentations/);
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
    assert.match(query, /searchMode: AUTO/);
    assert.match(query, /searchBreakdown\(first: 3\)/);
    assert.match(query, /artefacts\(first: 10\)/);
    assert.match(query, /score/);
    assert.match(query, /searchScore \{/);
    assert.match(query, /startLine/);
    assert.match(query, /endLine/);
    assert.match(query, /summary/);
    assert.match(query, /embeddingRepresentations/);
  });

  test('buildSearchQuery routes code-like queries to lexical mode', () => {
    const query = buildSearchQuery('  Method::HEAD  ', 5);

    assert.match(query, /search: "Method::HEAD"/);
    assert.match(query, /searchMode: LEXICAL/);
  });

  test('buildSelectionDetailsQuery requests artefact summaries and embeddings', () => {
    const query = buildSelectionDetailsQuery({
      path: 'src/main.ts',
      symbolFqn: 'src/main.ts::main',
    });

    assert.match(query, /selectArtefacts\(by: \{\s*symbolFqn: "src\/main\.ts::main"/);
    assert.match(query, /artefacts\(first: 1\)/);
    assert.match(query, /summary/);
    assert.match(query, /embeddingRepresentations/);
  });

  test('buildStageItemsQuery composes dependency and checkpoint stage lookups', () => {
    const dependencyQuery = buildStageItemsQuery(
      {
        path: 'src/main.ts',
        symbolFqn: 'src/main.ts::main',
      },
      {
        stage: 'dependencies',
        filterKey: 'calls',
        resultLimit: 20,
      },
    );
    const checkpointQuery = buildStageItemsQuery(
      {
        path: 'src/main.ts',
        lines: {
          start: 3,
          end: 9,
        },
      },
      {
        stage: 'checkpoints',
        resultLimit: 12,
      },
    );

    assert.match(dependencyQuery, /dependencies\(direction: BOTH, includeUnresolved: true, kind: CALLS\)/);
    assert.match(dependencyQuery, /toArtefact \{/);
    assert.match(checkpointQuery, /checkpoints \{/);
    assert.match(checkpointQuery, /items\(first: 12\)/);
    assert.match(checkpointQuery, /fileRelations \{/);
  });
});
