import { EmbeddingRepresentationKind, SidebarEmbeddingBadge, SelectionOverview } from './types';

interface OverviewCounts {
  checkpoints: number;
  dependencies: number;
  codeMatches: number;
  tests: number;
}

function toInteger(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

function pluralise(count: number, singular: string, plural = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value;
  }

  return `${value.slice(0, Math.max(0, maxLength - 3)).trimEnd()}...`;
}

export function normaliseSummaryText(summary?: string | null): string | undefined {
  if (typeof summary !== 'string') {
    return undefined;
  }

  const trimmed = summary.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

export function extractOverviewCounts(overview: SelectionOverview): OverviewCounts {
  return {
    checkpoints: toInteger(overview.checkpoints?.overview?.totalCount),
    dependencies: toInteger(overview.dependencies?.overview?.dependencies?.total),
    codeMatches: toInteger(overview.codeMatches?.overview?.counts?.total),
    tests: toInteger(overview.tests?.overview?.totalCoveringTests),
  };
}

export function formatOverviewSegments(overview: SelectionOverview): string[] {
  const counts = extractOverviewCounts(overview);
  const segments: string[] = [];

  if (counts.checkpoints > 0) {
    segments.push(pluralise(counts.checkpoints, 'checkpoint'));
  }

  if (counts.dependencies > 0) {
    segments.push(pluralise(counts.dependencies, 'dependency', 'dependencies'));
  }

  if (counts.codeMatches > 0) {
    segments.push(pluralise(counts.codeMatches, 'code match', 'code matches'));
  }

  if (counts.tests > 0) {
    segments.push(pluralise(counts.tests, 'test'));
  }

  return segments;
}

export function formatOverviewCodeLensTitle(overview: SelectionOverview): string {
  const segments = formatOverviewSegments(overview);

  if (segments.length === 0) {
    return 'Bitloops: no related data';
  }

  return `Bitloops: ${segments.join(', ')}`;
}

export function formatSummaryCodeLensTitle(summary?: string | null): string | undefined {
  const normalised = normaliseSummaryText(summary);
  if (!normalised) {
    return undefined;
  }

  return `Bitloops summary: ${truncateText(normalised, 110)}`;
}

export function buildEmbeddingBadges(
  embeddingRepresentations: EmbeddingRepresentationKind[] | undefined,
): SidebarEmbeddingBadge[] {
  const kinds = new Set(embeddingRepresentations ?? []);

  return [
    {
      label: 'Name',
      available: kinds.has('IDENTITY'),
    },
    {
      label: 'Code',
      available: kinds.has('CODE'),
    },
    {
      label: 'Summary',
      available: kinds.has('SUMMARY'),
    },
  ];
}
