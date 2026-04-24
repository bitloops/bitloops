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
    assert.equal(result.mode, 'AUTO');
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
    assert.equal(result.mode, 'AUTO');
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
              score: 4123,
              searchScore: {
                total: 4123,
                exact: 4094,
                fullText: 29,
                fuzzy: 0,
                semantic: 0,
                literalMatches: 3,
                exactCaseLiteralMatches: 3,
                phraseMatches: 0,
                exactCasePhraseMatches: 0,
                bodyLiteralMatches: 3,
                signatureLiteralMatches: 0,
                summaryLiteralMatches: 0,
              },
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
    assert.equal(result.artefacts[0].score, 4123);
    assert.equal(result.artefacts[0].searchScore?.exact, 4094);
    assert.equal(result.mode, 'AUTO');
  });

  test('parses search breakdown payloads for auto mode', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient([{
        selectArtefacts: {
          count: 2,
          artefacts: [
            {
              path: 'src/http.ts',
              symbolFqn: 'src/http.ts::handleHead',
              canonicalKind: 'FUNCTION',
              summary: 'Handles HEAD requests.',
              startLine: 4,
              endLine: 16,
            },
          ],
          searchBreakdown: {
            lexical: [
              {
                path: 'src/http.ts',
                symbolFqn: 'src/http.ts::stripBody',
                canonicalKind: 'FUNCTION',
                summary: 'Strips response bodies for HEAD requests.',
                score: 4380,
                searchScore: {
                  total: 4380,
                  exact: 4094,
                  fullText: 286,
                  fuzzy: 0,
                  semantic: 0,
                  literalMatches: 6,
                  exactCaseLiteralMatches: 6,
                  phraseMatches: 0,
                  exactCasePhraseMatches: 0,
                  bodyLiteralMatches: 6,
                  signatureLiteralMatches: 0,
                  summaryLiteralMatches: 0,
                },
                startLine: 20,
                endLine: 34,
              },
            ],
            identity: [],
            code: [
              {
                path: 'src/http.ts',
                symbolFqn: 'src/http.ts::handleHead',
                canonicalKind: 'FUNCTION',
                startLine: 4,
                endLine: 16,
              },
            ],
            summary: [],
          },
        },
      }]),
    );

    const result = await service.search('/repo', 'head request handler', 5);

    assert.equal(result.mode, 'AUTO');
    assert.equal(result.breakdown?.lexical.length, 1);
    assert.equal(result.breakdown?.lexical[0].symbolFqn, 'src/http.ts::stripBody');
    assert.equal(result.breakdown?.lexical[0].searchScore?.literalMatches, 6);
    assert.equal(result.breakdown?.code.length, 1);
    assert.deepEqual(result.breakdown?.identity, []);
  });

  test('marks code-like queries as lexical mode', async () => {
    const service = new BitloopsSearchService(
      new FakeQueryClient([{
        selectArtefacts: {
          count: 1,
          artefacts: [
            {
              path: 'src/http.ts',
              symbolFqn: 'src/http.ts::handleHead',
              canonicalKind: 'FUNCTION',
              startLine: 4,
              endLine: 16,
            },
          ],
        },
      }]),
    );

    const result = await service.search('/repo', 'Method::HEAD', 5);

    assert.equal(result.mode, 'LEXICAL');
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
    assert.match(client.queries[0], /\n      summary\n/);
    assert.match(client.queries[0], /embeddingRepresentations/);
    assert.doesNotMatch(client.queries[1], /\n      summary\n/);
    assert.doesNotMatch(client.queries[1], /embeddingRepresentations/);
  });
});
