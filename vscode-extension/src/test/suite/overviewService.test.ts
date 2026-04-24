import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import { BitloopsCliError, BitloopsQueryClient } from '../../bitloopsCli';
import { BitloopsOverviewService } from '../../overviewService';
import { SelectionTarget } from '../../types';

class FakeQueryClient implements BitloopsQueryClient {
  readonly queries: string[] = [];

  constructor(private readonly results: unknown[]) {}

  async executeGraphqlQuery<T>(_cwd: string, query: string): Promise<T> {
    this.queries.push(query);
    const next = this.results.shift();
    if (next instanceof Error) {
      throw next;
    }

    return next as T;
  }
}

function artefactTarget(): SelectionTarget {
  return {
    kind: 'artefact',
    workspaceFolderFsPath: '/repo',
    selector: {
      path: 'src/main.ts',
      symbolFqn: 'src/main.ts::main',
    },
    title: 'src/main.ts::main',
  };
}

suite('overviewService', () => {
  test('loads file overview and per-artefact overviews', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          count: 1,
          overview: {
            selectedArtefactCount: 1,
          },
          artefacts: [
            {
              path: 'src/main.ts',
              symbolFqn: 'src/main.ts::main',
              canonicalKind: 'FUNCTION',
              summary: 'Handles the main response path.',
              embeddingRepresentations: ['IDENTITY', 'SUMMARY'],
              startLine: 3,
              endLine: 9,
            },
          ],
        },
      },
      {
        artefact0: {
          overview: {
            selectedArtefactCount: 1,
            tests: {
              overview: {
                totalCoveringTests: 2,
              },
            },
          },
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadFileOverview('/repo', 'src/main.ts', 10);

    assert.equal(result.count, 1);
    assert.equal(result.artefacts.length, 1);
    assert.equal(result.artefacts[0].summary, 'Handles the main response path.');
    assert.deepEqual(result.artefacts[0].embeddingRepresentations, ['IDENTITY', 'SUMMARY']);
    assert.equal(result.artefacts[0].overview?.tests?.overview?.totalCoveringTests, 2);
    assert.equal(client.queries.length, 2);
  });

  test('loads artefact selection details with embeddings', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          count: 1,
          overview: {
            selectedArtefactCount: 1,
          },
          artefacts: [
            {
              path: 'src/main.ts',
              symbolFqn: 'src/main.ts::main',
              canonicalKind: 'FUNCTION',
              summary: 'Builds the response.',
              embeddingRepresentations: ['IDENTITY', 'CODE'],
              startLine: 3,
              endLine: 9,
            },
          ],
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadSelectionDetails('/repo', artefactTarget());

    assert.equal(result.count, 1);
    assert.equal(result.artefact?.summary, 'Builds the response.');
    assert.deepEqual(result.embeddingRepresentations, ['IDENTITY', 'CODE']);
  });

  test('loads file selection details without artefact rows', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          count: 3,
          overview: {
            selectedArtefactCount: 3,
          },
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadSelectionDetails('/repo', {
      kind: 'file',
      workspaceFolderFsPath: '/repo',
      selector: {
        path: 'src/main.ts',
      },
      title: 'src/main.ts',
    });

    assert.equal(result.count, 3);
    assert.equal(result.artefact, undefined);
    assert.deepEqual(result.embeddingRepresentations, []);
  });

  test('loads dependency, code-match, test, and checkpoint stage rows', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          dependencies: {
            items: [
              {
                id: 'dep-1',
                edgeKind: 'CALLS',
                fromArtefact: {
                  path: 'src/main.ts',
                  symbolFqn: 'src/main.ts::main',
                  canonicalKind: 'FUNCTION',
                  startLine: 3,
                  endLine: 9,
                },
                toArtefact: {
                  path: 'src/dep.ts',
                  symbolFqn: 'src/dep.ts::helper',
                  canonicalKind: 'FUNCTION',
                  startLine: 11,
                  endLine: 14,
                },
              },
            ],
          },
        },
      },
      {
        selectArtefacts: {
          codeMatches: {
            items: [
              {
                id: 'match-1',
                relationKind: 'similar_implementation',
                score: 0.88,
                sourceStartLine: 3,
                sourceEndLine: 9,
                targetStartLine: 11,
                targetEndLine: 19,
                sourceArtefact: {
                  path: 'src/main.ts',
                  symbolFqn: 'src/main.ts::main',
                  canonicalKind: 'FUNCTION',
                  startLine: 3,
                  endLine: 9,
                },
                targetArtefact: {
                  path: 'src/clone.ts',
                  symbolFqn: 'src/clone.ts::clone',
                  canonicalKind: 'FUNCTION',
                  startLine: 11,
                  endLine: 19,
                },
              },
            ],
          },
        },
      },
      {
        selectArtefacts: {
          tests: {
            items: [
              {
                artefact: {
                  artefactId: 'art-1',
                  name: 'main',
                  kind: 'function',
                  filePath: 'src/main.ts',
                  startLine: 3,
                  endLine: 9,
                },
                coveringTests: [
                  {
                    testId: 'test-1',
                    testName: 'main works',
                    suiteName: 'main suite',
                    filePath: 'tests/main.test.ts',
                    startLine: 4,
                    endLine: 8,
                    confidence: 0.91,
                  },
                ],
                summary: {
                  totalCoveringTests: 1,
                  dataSources: ['coverage'],
                },
              },
            ],
          },
        },
      },
      {
        selectArtefacts: {
          checkpoints: {
            items: [
              {
                id: 'checkpoint-1',
                agent: 'codex',
                eventTime: '2026-04-24T10:00:00Z',
                filesTouched: ['src/main.ts'],
                fileRelations: [
                  {
                    filepath: 'src/main.ts',
                    changeKind: 'MODIFIED',
                    pathAfter: 'src/main.ts',
                  },
                ],
              },
            ],
          },
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const dependencyRows = await service.loadStageItems(
      '/repo',
      artefactTarget(),
      'dependencies',
      20,
      'calls',
    );
    const codeMatchRows = await service.loadStageItems(
      '/repo',
      artefactTarget(),
      'codeMatches',
      20,
      'similar_implementation',
    );
    const testRows = await service.loadStageItems('/repo', artefactTarget(), 'tests', 20);
    const checkpointRows = await service.loadStageItems(
      '/repo',
      artefactTarget(),
      'checkpoints',
      20,
    );

    assert.equal(dependencyRows.rows[0].title, 'src/dep.ts::helper');
    assert.equal(dependencyRows.rows[0].navigationTarget?.path, 'src/dep.ts');
    assert.equal(codeMatchRows.rows[0].title, 'src/clone.ts::clone');
    assert.equal(codeMatchRows.rows[0].navigationTarget?.startLine, 11);
    assert.equal(testRows.rows[0].title, 'main works');
    assert.equal(testRows.rows[0].navigationTarget?.path, 'tests/main.test.ts');
    assert.equal(checkpointRows.rows[0].checkpointDetail?.files[0].path, 'src/main.ts');
    assert.equal(client.queries.length, 4);
  });

  test('retries file overview queries without optional artefact fields on older daemons', async () => {
    const client = new FakeQueryClient([
      new BitloopsCliError(
        'query failed',
        'query failed',
        'Unknown field "summary" on type "Artefact"\nUnknown field "embeddingRepresentations" on type "Artefact"',
      ),
      {
        selectArtefacts: {
          count: 0,
          overview: {},
          artefacts: [],
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadFileOverview('/repo', 'src/main.ts', 5);

    assert.equal(result.count, 0);
    assert.equal(client.queries.length, 2);
    assert.match(client.queries[0], /summary/);
    assert.match(client.queries[0], /embeddingRepresentations/);
    assert.doesNotMatch(client.queries[1], /summary/);
    assert.doesNotMatch(client.queries[1], /embeddingRepresentations/);
  });
});
