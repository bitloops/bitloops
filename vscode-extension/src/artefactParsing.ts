import {
  ArtefactSearchScore,
  BitloopsArtefact,
  EmbeddingRepresentationKind,
} from './types';

export function toNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

export function normaliseEmbeddingRepresentations(
  value: unknown,
): EmbeddingRepresentationKind[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const representations: EmbeddingRepresentationKind[] = [];

  for (const entry of value) {
    if (entry === 'IDENTITY' || entry === 'CODE' || entry === 'SUMMARY') {
      representations.push(entry);
    }
  }

  return representations;
}

function normaliseSearchScore(value: unknown): ArtefactSearchScore | undefined {
  if (!value || typeof value !== 'object') {
    return undefined;
  }

  const record = value as Record<string, unknown>;
  return {
    total: toNumber(record.total),
    exact: toNumber(record.exact),
    fullText: toNumber(record.fullText),
    fuzzy: toNumber(record.fuzzy),
    semantic: toNumber(record.semantic),
    literalMatches: toNumber(record.literalMatches),
    exactCaseLiteralMatches: toNumber(record.exactCaseLiteralMatches),
    phraseMatches: toNumber(record.phraseMatches),
    exactCasePhraseMatches: toNumber(record.exactCasePhraseMatches),
    bodyLiteralMatches: toNumber(record.bodyLiteralMatches),
    signatureLiteralMatches: toNumber(record.signatureLiteralMatches),
    summaryLiteralMatches: toNumber(record.summaryLiteralMatches),
  };
}

export function normaliseArtefact(value: unknown): BitloopsArtefact | undefined {
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
    embeddingRepresentations: normaliseEmbeddingRepresentations(
      record.embeddingRepresentations,
    ),
    score: toNumber(record.score),
    searchScore: normaliseSearchScore(record.searchScore),
    startLine: toNumber(record.startLine),
    endLine: toNumber(record.endLine),
  };
}
