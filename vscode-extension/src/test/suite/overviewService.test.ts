import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import { BitloopsQueryClient } from '../../bitloopsCli';
import { BitloopsOverviewService } from '../../overviewService';

class FakeQueryClient implements BitloopsQueryClient {
  readonly queries: string[] = [];

  constructor(private readonly results: unknown[]) {}

  async executeGraphqlQuery<T>(_cwd: string, query: string): Promise<T> {
    this.queries.push(query);
    const next = this.results.shift();
    return next as T;
  }
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
    assert.equal(result.artefacts[0].overview?.tests?.overview?.totalCoveringTests, 2);
    assert.equal(client.queries.length, 2);
  });

  test('handles files with no artefacts', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          count: 0,
          overview: {
            selectedArtefactCount: 0,
          },
          artefacts: [],
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadFileOverview('/repo', 'src/empty.ts', 10);

    assert.equal(result.count, 0);
    assert.deepEqual(result.artefacts, []);
    assert.equal(client.queries.length, 1);
  });

  test('falls back to path and line selectors when symbolFqn is missing', async () => {
    const client = new FakeQueryClient([
      {
        selectArtefacts: {
          count: 1,
          overview: {},
          artefacts: [
            {
              path: 'src/main.ts',
              canonicalKind: 'FUNCTION',
              startLine: 11,
              endLine: 19,
            },
          ],
        },
      },
      {
        artefact0: {
          overview: {
            selectedArtefactCount: 1,
          },
        },
      },
    ]);
    const service = new BitloopsOverviewService(client);

    const result = await service.loadFileOverview('/repo', 'src/main.ts', 10);

    assert.equal(result.artefacts.length, 1);
    assert.match(client.queries[1], /lines: \{ start: 11, end: 19 \}/);
  });
});
