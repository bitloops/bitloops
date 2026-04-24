import { BitloopsQueryClient } from './bitloopsCli';
import { buildActiveFileQuery, buildArtefactOverviewBatchQuery } from './queryBuilder';
import {
  ActiveFileSelectionData,
  ArtefactSelector,
  BitloopsArtefact,
  DocumentOverviewData,
  SelectionOverview,
} from './types';

interface SelectArtefactsOverviewResponse {
  overview?: SelectionOverview;
}

function toNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

function normaliseArtefact(value: unknown): BitloopsArtefact | undefined {
  if (!value || typeof value !== 'object') {
    return undefined;
  }

  const record = value as Record<string, unknown>;
  const path = typeof record.path === 'string' ? record.path : undefined;

  if (!path) {
    return undefined;
  }

  return {
    path,
    symbolFqn: typeof record.symbolFqn === 'string' ? record.symbolFqn : undefined,
    canonicalKind:
      typeof record.canonicalKind === 'string' ? record.canonicalKind : undefined,
    summary: typeof record.summary === 'string' ? record.summary : undefined,
    startLine: toNumber(record.startLine),
    endLine: toNumber(record.endLine),
  };
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

export class BitloopsOverviewService {
  constructor(private readonly client: BitloopsQueryClient) {}

  async loadFileOverview(
    cwd: string,
    relativePath: string,
    artefactLimit: number,
  ): Promise<DocumentOverviewData> {
    const fileResponse = await this.client.executeGraphqlQuery<unknown>(
      cwd,
      buildActiveFileQuery(relativePath, artefactLimit),
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
}
