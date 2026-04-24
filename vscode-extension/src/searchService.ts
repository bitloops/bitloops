import { ArtefactQuerySupport } from './artefactQuerySupport';
import { normaliseArtefact, toNumber } from './artefactParsing';
import { BitloopsQueryClient } from './bitloopsCli';
import { buildSearchQuery } from './queryBuilder';
import { BitloopsArtefact, SearchSelectionData } from './types';

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
  private readonly querySupport = new ArtefactQuerySupport();

  constructor(private readonly client: BitloopsQueryClient) {}

  async search(cwd: string, search: string, resultLimit: number): Promise<SearchSelectionData> {
    const trimmed = search.trim();
    if (!trimmed) {
      return {
        count: 0,
        artefacts: [],
      };
    }

    const response = await this.querySupport.executeWithFallback<unknown>(
      this.client,
      cwd,
      (fieldSupport) => buildSearchQuery(trimmed, resultLimit, fieldSupport),
    );

    return extractSelectionRoot(response);
  }
}
