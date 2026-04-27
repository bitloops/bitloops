import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import {
  buildEmbeddingBadges,
  extractOverviewCounts,
  formatOverviewCodeLensTitle,
  formatOverviewSegments,
  formatSummaryCodeLensTitle,
} from '../../overviewFormatter';
import { SelectionOverview } from '../../types';

const sampleOverview: SelectionOverview = {
  selectedArtefactCount: 2,
  checkpoints: {
    overview: {
      totalCount: 1,
      latestAt: '2026-04-01T10:00:00Z',
      agents: ['codex'],
    },
  },
  dependencies: {
    overview: {
      dependencies: {
        total: 3,
        incoming: 1,
        outgoing: 2,
        kindCounts: {
          calls: 2,
          imports: 1,
        },
      },
    },
  },
  codeMatches: {
    overview: {
      counts: {
        total: 4,
        similar_implementation: 4,
      },
    },
  },
  tests: {
    overview: {
      matchedArtefactCount: 2,
      totalCoveringTests: 5,
    },
  },
};

suite('overviewFormatter', () => {
  test('extractOverviewCounts reads the current stage counts', () => {
    assert.deepEqual(extractOverviewCounts(sampleOverview), {
      checkpoints: 1,
      dependencies: 3,
      codeMatches: 4,
      tests: 5,
    });
  });

  test('formatOverviewSegments keeps the stage order stable', () => {
    assert.deepEqual(formatOverviewSegments(sampleOverview), [
      '1 checkpoint',
      '3 dependencies',
      '4 code matches',
      '5 tests',
    ]);
  });

  test('formatOverviewCodeLensTitle falls back to no related data when empty', () => {
    assert.equal(formatOverviewCodeLensTitle({}), 'Bitloops: no related data');
  });

  test('formatSummaryCodeLensTitle renders the semantic summary text', () => {
    assert.equal(
      formatSummaryCodeLensTitle('Builds the API response payload for the route.'),
      'Bitloops summary: Builds the API response payload for the route.',
    );
  });

  test('buildEmbeddingBadges marks the available representations', () => {
    assert.deepEqual(buildEmbeddingBadges(['IDENTITY', 'SUMMARY']), [
      {
        label: 'Name',
        available: true,
      },
      {
        label: 'Code',
        available: false,
      },
      {
        label: 'Summary',
        available: true,
      },
    ]);
  });
});
