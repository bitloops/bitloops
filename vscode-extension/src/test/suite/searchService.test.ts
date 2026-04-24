import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import { BitloopsCliError, BitloopsQueryClient } from '../../bitloopsCli';
import { BitloopsSearchService } from '../../searchService';

class FakeQueryClient implements BitloopsQueryClient {
  readonly queries: string[] = [];

  constructor(private readonly responses: unknown[]) {}

  async executeGraphqlQuery<T>(_cwd: string, query: string): Promise<T> {
    this.queries.push(query);
    const next = this.responses.shift();
    if (next instanceof Error) {
      throw next;
    }

    return next as T;
  }
}

suite('searchService', () => {
  test('returns zero results for empty search text', async () => {
    const service = new BitloopsSearchService(new FakeQueryClient([{}]));

    const result = await service.search('/repo', '   ', 10);

    assert.equal(result.count, 0);
    assert.deepEqual(result.artefacts, []);
  });

  test('returns zero-result payloads from DevQL', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient([{
        selectArtefacts: {
          count: 0,
          artefacts: [],
        },
      }]),
    );

    const result = await service.search('/repo', 'http', 10);

    assert.equal(result.count, 0);
    assert.deepEqual(result.artefacts, []);
  });

  test('returns partial and capped results without reshaping the reported count', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient([{
        selectArtefacts: {
          count: 12,
          artefacts: [
            {
              path: 'src/a.ts',
              symbolFqn: 'src/a.ts::a',
              canonicalKind: 'FUNCTION',
              summary: 'Builds the first handler response.',
              embeddingRepresentations: ['IDENTITY', 'CODE'],
              startLine: 1,
              endLine: 4,
            },
            {
              path: 'src/b.ts',
              symbolFqn: 'src/b.ts::b',
              canonicalKind: 'FUNCTION',
              summary: 'Builds the second handler response.',
              startLine: 5,
              endLine: 7,
            },
          ],
        },
      }]),
    );

    const result = await service.search('/repo', 'handler', 2);

    assert.equal(result.count, 12);
    assert.equal(result.artefacts.length, 2);
    assert.equal(result.artefacts[0].symbolFqn, 'src/a.ts::a');
    assert.equal(result.artefacts[0].summary, 'Builds the first handler response.');
    assert.deepEqual(result.artefacts[0].embeddingRepresentations, ['IDENTITY', 'CODE']);
  });

  test('retries without optional artefact fields when the daemon schema is older', async () => {
    const client = new FakeQueryClient([
      new BitloopsCliError(
        'query failed',
        'query failed',
        'Error: Unknown field "embeddingRepresentations" on type "Artefact"\nError: Unknown field "summary" on type "Artefact"',
      ),
      {
        selectArtefacts: {
          count: 1,
          artefacts: [
            {
              path: 'src/a.ts',
              symbolFqn: 'src/a.ts::a',
              canonicalKind: 'FUNCTION',
              startLine: 1,
              endLine: 2,
            },
          ],
        },
      },
    ]);
    const service = new BitloopsSearchService(client);

    const result = await service.search('/repo', 'handler', 5);

    assert.equal(result.count, 1);
    assert.equal(client.queries.length, 2);
    assert.match(client.queries[0], /summary/);
    assert.match(client.queries[0], /embeddingRepresentations/);
    assert.doesNotMatch(client.queries[1], /summary/);
    assert.doesNotMatch(client.queries[1], /embeddingRepresentations/);
  });
});
