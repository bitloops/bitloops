import { ArtefactQuerySupport } from './artefactQuerySupport';
import { normaliseArtefact, toNumber } from './artefactParsing';
import { BitloopsQueryClient } from './bitloopsCli';
import {
  buildActiveFileQuery,
  buildArtefactOverviewBatchQuery,
  buildSelectionDetailsQuery,
  buildSelectionOverviewQuery,
  buildStageItemsQuery,
} from './queryBuilder';
import {
  ActiveFileSelectionData,
  ArtefactSelector,
  BitloopsArtefact,
  DocumentOverviewData,
  EditorNavigationTarget,
  SelectionDetails,
  SelectionOverview,
  SelectionTarget,
  SidebarCheckpointFile,
  SidebarCheckpointDetail,
  SidebarStageRow,
  StageItemsResult,
  StageKind,
} from './types';

interface SelectArtefactsOverviewResponse {
  overview?: SelectionOverview;
}

function extractSelectionRoot(response: unknown): ActiveFileSelectionData {
  const root = response as {
    selectArtefacts?: {
      count?: unknown;
      overview?: SelectionOverview;
      artefacts?: unknown[];
    };
  };

  return {
    count: toNumber(root.selectArtefacts?.count),
    overview: root.selectArtefacts?.overview ?? {},
    artefacts: (root.selectArtefacts?.artefacts ?? [])
      .map(normaliseArtefact)
      .filter((artefact): artefact is BitloopsArtefact => Boolean(artefact)),
  };
}

function toSelector(artefact: BitloopsArtefact): ArtefactSelector {
  if (artefact.symbolFqn && artefact.symbolFqn.trim().length > 0) {
    return {
      path: artefact.path,
      symbolFqn: artefact.symbolFqn,
    };
  }

  return {
    path: artefact.path,
    lines: {
      start: artefact.startLine,
      end: artefact.endLine,
    },
  };
}

function toEditorNavigationTarget(
  artefact: Pick<BitloopsArtefact, 'path' | 'startLine' | 'endLine' | 'symbolFqn' | 'canonicalKind'>,
): EditorNavigationTarget {
  return {
    path: artefact.path,
    startLine: artefact.startLine,
    endLine: artefact.endLine,
    symbolFqn: artefact.symbolFqn,
    canonicalKind: artefact.canonicalKind,
  };
}

function selectionMatchesArtefact(target: SelectionTarget, artefact: BitloopsArtefact): boolean {
  if (target.selector.symbolFqn && artefact.symbolFqn) {
    return target.selector.symbolFqn === artefact.symbolFqn;
  }

  if (target.selector.path !== artefact.path) {
    return false;
  }

  if (!target.selector.lines) {
    return true;
  }

  return (
    target.selector.lines.start === artefact.startLine &&
    target.selector.lines.end === artefact.endLine
  );
}

function formatConfidence(value: unknown): string | undefined {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return undefined;
  }

  return value.toFixed(2);
}

function parseCheckpointRows(items: unknown[]): SidebarStageRow[] {
  return items.flatMap((item, index) => {
    if (!item || typeof item !== 'object') {
      return [];
    }

    const record = item as Record<string, unknown>;
    const filesTouched = Array.isArray(record.filesTouched)
      ? record.filesTouched.filter((entry): entry is string => typeof entry === 'string')
      : [];
    let fileRelations: SidebarCheckpointFile[];
    if (Array.isArray(record.fileRelations)) {
      fileRelations = record.fileRelations
        .filter((entry): entry is Record<string, unknown> => Boolean(entry) && typeof entry === 'object')
        .reduce<SidebarCheckpointFile[]>((files, entry, fileIndex) => {
          const filepath =
            typeof entry.pathAfter === 'string' && entry.pathAfter.trim().length > 0
              ? entry.pathAfter
              : typeof entry.filepath === 'string'
                ? entry.filepath
                : typeof entry.pathBefore === 'string'
                  ? entry.pathBefore
                  : undefined;
          if (!filepath) {
            return files;
          }

          files.push({
            id: `checkpoint-file-${index}-${fileIndex}`,
            label: filepath,
            path: filepath,
            changeKind:
              typeof entry.changeKind === 'string' ? entry.changeKind : undefined,
          });
          return files;
        }, []);
    } else {
      fileRelations = filesTouched.map((path, fileIndex) => ({
        id: `checkpoint-file-${index}-${fileIndex}`,
        label: path,
        path,
      }));
    }

    const metadata = [
      typeof record.agent === 'string' && record.agent.trim().length > 0
        ? `Agent: ${record.agent}`
        : undefined,
      typeof record.eventTime === 'string' ? `Event: ${record.eventTime}` : undefined,
      typeof record.commitSha === 'string' && record.commitSha.trim().length > 0
        ? `Commit: ${record.commitSha}`
        : undefined,
      typeof record.branch === 'string' && record.branch.trim().length > 0
        ? `Branch: ${record.branch}`
        : undefined,
      typeof record.strategy === 'string' && record.strategy.trim().length > 0
        ? `Strategy: ${record.strategy}`
        : undefined,
    ].filter((entry): entry is string => Boolean(entry));

    const detail: SidebarCheckpointDetail = {
      id: `checkpoint-${index}`,
      title:
        typeof record.id === 'string' ? record.id : `Checkpoint ${index + 1}`,
      description:
        typeof record.firstPromptPreview === 'string'
          ? record.firstPromptPreview
          : undefined,
      metadata,
      files: fileRelations,
    };

    return [
      {
        id: detail.id,
        title:
          typeof record.agent === 'string' && record.agent.trim().length > 0
            ? record.agent
            : detail.title,
        description:
          typeof record.eventTime === 'string' ? record.eventTime : undefined,
        detail:
          filesTouched.length > 0
            ? `${filesTouched.length} touched file${filesTouched.length === 1 ? '' : 's'}`
            : undefined,
        checkpointDetail: detail,
      },
    ];
  });
}

function parseDependencyRows(items: unknown[]): SidebarStageRow[] {
  return items.flatMap((item, index) => {
    if (!item || typeof item !== 'object') {
      return [];
    }

    const record = item as Record<string, unknown>;
    const fromArtefact = normaliseArtefact(record.fromArtefact);
    const toArtefact = normaliseArtefact(record.toArtefact);
    const unresolvedSymbol =
      typeof record.toSymbolRef === 'string' ? record.toSymbolRef : undefined;
    const lineStart = toNumber(record.startLine);
    const lineEnd = toNumber(record.endLine);

    if (toArtefact) {
      return [
        {
          id: `dependency-${index}`,
          title: toArtefact.symbolFqn ?? toArtefact.path,
          description: toArtefact.path,
          detail:
            typeof record.edgeKind === 'string'
              ? record.edgeKind.toLowerCase()
              : undefined,
          navigationTarget: toEditorNavigationTarget(toArtefact),
        },
      ];
    }

    if (!fromArtefact) {
      return [];
    }

    return [
      {
        id: `dependency-${index}`,
        title: unresolvedSymbol ?? 'Unresolved dependency',
        description: fromArtefact.path,
        detail:
          typeof record.edgeKind === 'string'
            ? `${record.edgeKind.toLowerCase()} · unresolved`
            : 'Unresolved dependency',
        navigationTarget: {
          path: fromArtefact.path,
          startLine: lineStart || fromArtefact.startLine,
          endLine: lineEnd || fromArtefact.endLine,
          symbolFqn: fromArtefact.symbolFqn,
          canonicalKind: fromArtefact.canonicalKind,
        },
      },
    ];
  });
}

function parseCodeMatchRows(
  target: SelectionTarget,
  items: unknown[],
): SidebarStageRow[] {
  return items.flatMap((item, index) => {
    if (!item || typeof item !== 'object') {
      return [];
    }

    const record = item as Record<string, unknown>;
    const sourceArtefact = normaliseArtefact(record.sourceArtefact);
    const targetArtefact = normaliseArtefact(record.targetArtefact);
    if (!sourceArtefact || !targetArtefact) {
      return [];
    }

    const selectedIsSource = selectionMatchesArtefact(target, sourceArtefact);
    const otherArtefact = selectedIsSource ? targetArtefact : sourceArtefact;
    const startLine = selectedIsSource
      ? toNumber(record.targetStartLine) || otherArtefact.startLine
      : toNumber(record.sourceStartLine) || otherArtefact.startLine;
    const endLine = selectedIsSource
      ? toNumber(record.targetEndLine) || otherArtefact.endLine
      : toNumber(record.sourceEndLine) || otherArtefact.endLine;
    const relationKind =
      typeof record.relationKind === 'string' ? record.relationKind : 'code match';
    const score = formatConfidence(record.score);

    return [
      {
        id: `code-match-${index}`,
        title: otherArtefact.symbolFqn ?? otherArtefact.path,
        description: otherArtefact.path,
        detail: score ? `${relationKind.replace(/_/g, ' ')} · score ${score}` : relationKind,
        navigationTarget: {
          ...toEditorNavigationTarget(otherArtefact),
          startLine,
          endLine,
        },
      },
    ];
  });
}

function parseTestRows(items: unknown[]): SidebarStageRow[] {
  return items.flatMap((item, index) => {
    if (!item || typeof item !== 'object') {
      return [];
    }

    const record = item as Record<string, unknown>;
    const summary = (record.summary ?? {}) as Record<string, unknown>;
    const tests = Array.isArray(record.coveringTests) ? record.coveringTests : [];

    return tests.flatMap((testValue, testIndex) => {
      if (!testValue || typeof testValue !== 'object') {
        return [];
      }

      const test = testValue as Record<string, unknown>;
      const filePath = typeof test.filePath === 'string' ? test.filePath : undefined;
      if (!filePath) {
        return [];
      }

      const confidence = formatConfidence(test.confidence);
      const suiteName =
        typeof test.suiteName === 'string' && test.suiteName.trim().length > 0
          ? test.suiteName
          : undefined;
      const dataSources = Array.isArray(summary.dataSources)
        ? summary.dataSources.filter((entry): entry is string => typeof entry === 'string')
        : [];

      return [
        {
          id: `test-${index}-${testIndex}`,
          title:
            typeof test.testName === 'string' ? test.testName : `Covering test ${testIndex + 1}`,
          description: suiteName ?? filePath,
          detail: [
            confidence ? `confidence ${confidence}` : undefined,
            typeof summary.totalCoveringTests === 'number'
              ? `${summary.totalCoveringTests} covering`
              : undefined,
            dataSources.length > 0 ? dataSources.join(', ') : undefined,
          ]
            .filter((entry): entry is string => Boolean(entry))
            .join(' · '),
          navigationTarget: {
            path: filePath,
            startLine: toNumber(test.startLine),
            endLine: toNumber(test.endLine),
          },
        },
      ];
    });
  });
}

function parseStageRows(
  target: SelectionTarget,
  stage: StageKind,
  items: unknown[],
): SidebarStageRow[] {
  switch (stage) {
    case 'checkpoints':
      return parseCheckpointRows(items);
    case 'dependencies':
      return parseDependencyRows(items);
    case 'codeMatches':
      return parseCodeMatchRows(target, items);
    case 'tests':
      return parseTestRows(items);
  }
}

function emptyStageMessage(stage: StageKind, filterKey?: string): string {
  switch (stage) {
    case 'checkpoints':
      return 'No checkpoints found for this selection.';
    case 'dependencies':
      return filterKey
        ? `No ${filterKey.replace(/_/g, ' ')} dependencies found for this selection.`
        : 'No dependencies found for this selection.';
    case 'codeMatches':
      return filterKey
        ? `No ${filterKey.replace(/_/g, ' ')} code matches found for this selection.`
        : 'No code matches found for this selection.';
    case 'tests':
      return 'No tests found for this selection.';
  }
}

export class BitloopsOverviewService {
  private readonly querySupport = new ArtefactQuerySupport();

  constructor(private readonly client: BitloopsQueryClient) {}

  async loadFileOverview(
    cwd: string,
    relativePath: string,
    artefactLimit: number,
  ): Promise<DocumentOverviewData> {
    const fileResponse = await this.querySupport.executeWithFallback<unknown>(
      this.client,
      cwd,
      (fieldSupport) => buildActiveFileQuery(relativePath, artefactLimit, fieldSupport),
    );
    const selection = extractSelectionRoot(fileResponse);
    const selectors = selection.artefacts.map(toSelector);
    const batch = buildArtefactOverviewBatchQuery(selectors);
    const perArtefactOverviews = new Map<string, SelectionOverview>();

    if (batch) {
      const batchResponse = await this.client.executeGraphqlQuery<
        Record<string, SelectArtefactsOverviewResponse>
      >(cwd, batch.query);

      for (const alias of batch.aliases) {
        const overview = batchResponse[alias.alias]?.overview;
        if (overview) {
          perArtefactOverviews.set(alias.alias, overview);
        }
      }
    }

    return {
      count: selection.count,
      path: relativePath,
      overview: selection.overview,
      artefacts: selection.artefacts.map((artefact, index) => ({
        ...artefact,
        overview: batch ? perArtefactOverviews.get(batch.aliases[index].alias) : undefined,
      })),
    };
  }

  async loadSelectionDetails(
    cwd: string,
    target: SelectionTarget,
  ): Promise<SelectionDetails> {
    if (target.kind === 'file') {
      const response = await this.client.executeGraphqlQuery<unknown>(
        cwd,
        buildSelectionOverviewQuery(target.selector),
      );
      const selection = extractSelectionRoot(response);
      return {
        count: selection.count,
        overview: selection.overview,
        embeddingRepresentations: [],
      };
    }

    const response = await this.querySupport.executeWithFallback<unknown>(
      this.client,
      cwd,
      (fieldSupport) => buildSelectionDetailsQuery(target.selector, fieldSupport),
    );
    const selection = extractSelectionRoot(response);
    const artefact = selection.artefacts[0] ?? target.preview?.artefact;

    return {
      count: selection.count,
      overview: selection.overview,
      artefact,
      embeddingRepresentations:
        artefact?.embeddingRepresentations ?? target.preview?.embeddingRepresentations ?? [],
    };
  }

  async loadStageItems(
    cwd: string,
    target: SelectionTarget,
    stage: StageKind,
    resultLimit: number,
    filterKey?: string,
  ): Promise<StageItemsResult> {
    const response = await this.client.executeGraphqlQuery<{
      selectArtefacts?: Record<string, { items?: unknown[] }>;
    }>(
      cwd,
      buildStageItemsQuery(target.selector, {
        stage,
        filterKey,
        resultLimit,
      }),
    );

    const stageItems =
      response.selectArtefacts?.[stage]?.items && Array.isArray(response.selectArtefacts[stage].items)
        ? response.selectArtefacts[stage].items
        : [];

    return {
      stage,
      filterKey,
      rows: parseStageRows(target, stage, stageItems),
      emptyMessage: emptyStageMessage(stage, filterKey),
    };
  }
}
