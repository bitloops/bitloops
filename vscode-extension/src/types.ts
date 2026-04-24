export interface BitloopsArtefact {
  path: string;
  symbolFqn?: string | null;
  canonicalKind?: string | null;
  summary?: string | null;
  startLine: number;
  endLine: number;
}

export interface LineRange {
  start: number;
  end: number;
}

export interface ArtefactSelector {
  path: string;
  symbolFqn?: string | null;
  lines?: LineRange;
}

export interface CheckpointOverview {
  totalCount?: number;
  latestAt?: string | null;
  agents?: string[];
}

export interface CodeMatchesOverview {
  counts?: Record<string, number>;
  expandHint?: Record<string, unknown>;
}

export interface DependencyOverview {
  dependencies?: {
    selectedArtefact?: number;
    total?: number;
    incoming?: number;
    outgoing?: number;
    kindCounts?: Record<string, number>;
  };
  expandHint?: Record<string, unknown>;
}

export interface TestsOverview {
  selectedArtefactCount?: number;
  matchedArtefactCount?: number;
  totalCoveringTests?: number;
  expandHint?: Record<string, unknown>;
}

export interface SelectionOverviewStage<TOverview> {
  overview?: TOverview;
  schema?: string | null;
  expandHint?: Record<string, unknown>;
}

export interface SelectionOverview {
  selectedArtefactCount?: number;
  checkpoints?: SelectionOverviewStage<CheckpointOverview>;
  codeMatches?: SelectionOverviewStage<CodeMatchesOverview>;
  dependencies?: SelectionOverviewStage<DependencyOverview>;
  tests?: SelectionOverviewStage<TestsOverview>;
  [key: string]: unknown;
}

export interface ActiveFileSelectionData {
  count: number;
  overview: SelectionOverview;
  artefacts: BitloopsArtefact[];
}

export interface DocumentArtefactOverview extends BitloopsArtefact {
  overview?: SelectionOverview;
}

export interface DocumentOverviewData {
  count: number;
  path: string;
  overview: SelectionOverview;
  artefacts: DocumentArtefactOverview[];
}

export interface SearchSelectionData {
  count: number;
  artefacts: BitloopsArtefact[];
}

export interface OverviewDetailRow {
  label: string;
  description?: string;
}

export interface OverviewCommandArgs {
  title: string;
  overview: SelectionOverview;
  summary?: string | null;
}

export interface OpenSearchResultArgs {
  workspaceFolderFsPath: string;
  artefact: BitloopsArtefact;
}
