import { BitloopsQueryClient } from './bitloopsCli';
import { buildSearchQuery } from './queryBuilder';
import { BitloopsArtefact, SearchSelectionData } from './types';

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

function extractSelectionRoot(response: unknown): SearchSelectionData {
  const root = response as {
    selectArtefacts?: {
      count?: unknown;
      artefacts?: unknown[];
    };
  };

  return {
    count: toNumber(root.selectArtefacts?.count),
    artefacts: (root.selectArtefacts?.artefacts ?? [])
      .map(normaliseArtefact)
      .filter((artefact): artefact is BitloopsArtefact => Boolean(artefact)),
  };
}

export class BitloopsSearchService {
  constructor(private readonly client: BitloopsQueryClient) {}

  async search(cwd: string, search: string, resultLimit: number): Promise<SearchSelectionData> {
    const trimmed = search.trim();
    if (!trimmed) {
      return {
        count: 0,
        artefacts: [],
      };
    }

    const response = await this.client.executeGraphqlQuery<unknown>(
      cwd,
      buildSearchQuery(trimmed, resultLimit),
    );

    return extractSelectionRoot(response);
  }
}
