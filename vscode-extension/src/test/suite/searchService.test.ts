import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import { BitloopsQueryClient } from '../../bitloopsCli';
import { BitloopsSearchService } from '../../searchService';

class FakeQueryClient implements BitloopsQueryClient {
  constructor(private readonly response: unknown) {}

  async executeGraphqlQuery<T>(): Promise<T> {
    return this.response as T;
  }
}

suite('searchService', () => {
  test('returns zero results for empty search text', async () => {
    const service = new BitloopsSearchService(new FakeQueryClient({}));

    const result = await service.search('/repo', '   ', 10);

    assert.equal(result.count, 0);
    assert.deepEqual(result.artefacts, []);
  });

  test('returns zero-result payloads from DevQL', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient({
        selectArtefacts: {
          count: 0,
          artefacts: [],
        },
      }),
    );

    const result = await service.search('/repo', 'http', 10);

    assert.equal(result.count, 0);
    assert.deepEqual(result.artefacts, []);
  });

  test('returns partial and capped results without reshaping the reported count', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient({
        selectArtefacts: {
          count: 12,
          artefacts: [
            {
              path: 'src/a.ts',
              symbolFqn: 'src/a.ts::a',
              canonicalKind: 'FUNCTION',
              summary: 'Builds the first handler response.',
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
      }),
    );

    const result = await service.search('/repo', 'handler', 2);

    assert.equal(result.count, 12);
    assert.equal(result.artefacts.length, 2);
    assert.equal(result.artefacts[0].symbolFqn, 'src/a.ts::a');
    assert.equal(result.artefacts[0].summary, 'Builds the first handler response.');
  });
});
