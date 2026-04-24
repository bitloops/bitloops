import { ArtefactQuerySupport } from './artefactQuerySupport';
import { normaliseArtefact, toNumber } from './artefactParsing';
import { BitloopsQueryClient } from './bitloopsCli';
import { buildSearchQuery, inferSearchMode } from './queryBuilder';
import { BitloopsArtefact, SearchBreakdownData, SearchSelectionData } from './types';

function normaliseArtefacts(value: unknown[] | undefined): BitloopsArtefact[] {
  return (value ?? [])
    .map(normaliseArtefact)
    .filter((artefact): artefact is BitloopsArtefact => Boolean(artefact));
}

function extractSearchBreakdown(value: unknown): SearchBreakdownData | undefined {
  const breakdown = value as {
    lexical?: unknown[];
    identity?: unknown[];
    code?: unknown[];
    summary?: unknown[];
  } | null | undefined;

  if (!breakdown || typeof breakdown !== 'object') {
    return undefined;
  }

  return {
    lexical: normaliseArtefacts(breakdown.lexical),
    identity: normaliseArtefacts(breakdown.identity),
    code: normaliseArtefacts(breakdown.code),
    summary: normaliseArtefacts(breakdown.summary),
  };
}

function extractSelectionRoot(response: unknown, search: string): SearchSelectionData {
  const root = response as {
    selectArtefacts?: {
      count?: unknown;
      artefacts?: unknown[];
      searchBreakdown?: unknown;
    };
  };

  return {
    count: toNumber(root.selectArtefacts?.count),
    artefacts: normaliseArtefacts(root.selectArtefacts?.artefacts),
    mode: inferSearchMode(search),
    breakdown: extractSearchBreakdown(root.selectArtefacts?.searchBreakdown),
  };
}

export class BitloopsSearchService {
  private readonly querySupport = new ArtefactQuerySupport();

  constructor(private readonly client: BitloopsQueryClient) {}

  async search(cwd: string, search: string, resultLimit: number): Promise<SearchSelectionData> {
    const trimmed = search.trim();
    if (!trimmed) {
      return {
        count: 0,
        artefacts: [],
        mode: inferSearchMode(trimmed),
      };
    }

    const response = await this.querySupport.executeWithFallback<unknown>(
      this.client,
      cwd,
      (fieldSupport) => buildSearchQuery(trimmed, resultLimit, fieldSupport),
    );

    return extractSelectionRoot(response, trimmed);
  }
}
