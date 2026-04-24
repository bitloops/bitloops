import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import { BitloopsSidebarController } from '../../sidebarController';

suite('sidebarController', () => {
  test('tracks search results, selection reveal, and breadcrumb navigation', () => {
    const controller = new BitloopsSidebarController();

    controller.setSearchResults(
      'handler',
      '/repo',
      [
        {
          path: 'src/main.ts',
          symbolFqn: 'src/main.ts::main',
          canonicalKind: 'FUNCTION',
          summary: 'Builds the main handler response.',
          embeddingRepresentations: ['IDENTITY'],
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
          startLine: 3,
          endLine: 9,
        },
      ],
      1,
      'AUTO',
      {
        lexical: [
          {
            path: 'src/main.ts',
            symbolFqn: 'src/main.ts::stripBody',
            canonicalKind: 'FUNCTION',
            summary: 'Strips the response body.',
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
            startLine: 11,
            endLine: 19,
          },
        ],
        identity: [],
        code: [],
        summary: [],
      },
    );

    const target = controller.getSearchResultTarget('result-0');
    assert.ok(target);
    assert.ok(controller.getSearchResultTarget('breakdown-lexical-0'));
    controller.revealSelection(target!);
    controller.applySelectionDetails(target!, {
      count: 1,
      overview: {
        dependencies: {
          overview: {
            dependencies: {
              total: 2,
              kindCounts: {
                calls: 2,
              },
            },
          },
        },
      },
      artefact: {
        path: 'src/main.ts',
        symbolFqn: 'src/main.ts::main',
        canonicalKind: 'FUNCTION',
        summary: 'Builds the main handler response.',
        embeddingRepresentations: ['IDENTITY', 'SUMMARY'],
        startLine: 3,
        endLine: 9,
      },
      embeddingRepresentations: ['IDENTITY', 'SUMMARY'],
    });

    const selectionState = controller.viewState();
    assert.equal(selectionState.searchMode, 'AUTO');
    assert.equal(selectionState.searchSections.length, 1);
    assert.equal(selectionState.searchSections[0].title, 'Lexical');
    assert.equal(selectionState.searchSections[0].results[0].title, 'src/main.ts::stripBody');
    assert.equal(selectionState.searchSections[0].results[0].scoreLabel, 'score 4380');
    assert.equal(
      selectionState.searchSections[0].results[0].scoreBreakdownLabel,
      'exact 4094 · text 286',
    );
    assert.equal(
      selectionState.searchSections[0].results[0].matchBreakdownLabel,
      'literals 6 · exact-case 6 · body 6',
    );
    assert.equal(selectionState.selection?.title, 'src/main.ts::main');
    assert.equal(selectionState.breadcrumbs.length, 2);
    assert.equal(selectionState.selection?.badges[0].available, true);
    assert.equal(selectionState.selection?.badges[1].available, false);
    assert.equal(selectionState.selection?.chips[1].count, 2);

    controller.navigateToBreadcrumb('results');
    assert.equal(controller.viewState().selection, undefined);
  });

  test('opens checkpoint detail and backs out through the inspector stack', () => {
    const controller = new BitloopsSidebarController();

    controller.revealSelection({
      kind: 'artefact',
      workspaceFolderFsPath: '/repo',
      selector: {
        path: 'src/main.ts',
        symbolFqn: 'src/main.ts::main',
      },
      title: 'src/main.ts::main',
    });
    controller.applySelectionDetails(
      {
        kind: 'artefact',
        workspaceFolderFsPath: '/repo',
        selector: {
          path: 'src/main.ts',
          symbolFqn: 'src/main.ts::main',
        },
        title: 'src/main.ts::main',
      },
      {
        count: 1,
        overview: {},
        embeddingRepresentations: [],
      },
    );
    controller.setActiveStage({
      stage: 'checkpoints',
      rows: [
        {
          id: 'checkpoint-1',
          title: 'codex',
          checkpointDetail: {
            id: 'checkpoint-1',
            title: 'checkpoint-1',
            metadata: ['Agent: codex'],
            files: [
              {
                id: 'file-1',
                label: 'src/main.ts',
                path: 'src/main.ts',
              },
            ],
          },
        },
      ],
      emptyMessage: 'No checkpoints.',
    });

    const row = controller.getStageRow('checkpoint-1');
    assert.ok(row?.checkpointDetail);
    controller.openCheckpointDetail(row!.checkpointDetail!);
    assert.equal(controller.viewState().checkpoint?.files[0].path, 'src/main.ts');

    controller.back();
    assert.equal(controller.viewState().checkpoint, undefined);
    assert.ok(controller.viewState().stage);

    controller.back();
    assert.equal(controller.viewState().stage, undefined);
    assert.ok(controller.viewState().selection);
  });
});
