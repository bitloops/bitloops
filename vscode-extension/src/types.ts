export interface BitloopsArtefact {
  path: string;
  symbolFqn?: string | null;
  canonicalKind?: string | null;
  summary?: string | null;
  embeddingRepresentations?: EmbeddingRepresentationKind[];
  score?: number;
  searchScore?: ArtefactSearchScore;
  startLine: number;
  endLine: number;
}

export type EmbeddingRepresentationKind = 'IDENTITY' | 'CODE' | 'SUMMARY';
export type SearchMode = 'AUTO' | 'IDENTITY' | 'CODE' | 'SUMMARY' | 'LEXICAL';

export interface ArtefactSearchScore {
  total: number;
  exact: number;
  fullText: number;
  fuzzy: number;
  semantic: number;
  literalMatches: number;
  exactCaseLiteralMatches: number;
  phraseMatches: number;
  exactCasePhraseMatches: number;
  bodyLiteralMatches: number;
  signatureLiteralMatches: number;
  summaryLiteralMatches: number;
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

export interface SearchBreakdownData {
  lexical: BitloopsArtefact[];
  identity: BitloopsArtefact[];
  code: BitloopsArtefact[];
  summary: BitloopsArtefact[];
}

export interface SearchSelectionData {
  count: number;
  artefacts: BitloopsArtefact[];
  mode: SearchMode;
  breakdown?: SearchBreakdownData;
}

export type SelectionKind = 'artefact' | 'file';
export type StageKind = 'checkpoints' | 'dependencies' | 'codeMatches' | 'tests';

export interface SelectionDetails {
  count: number;
  overview: SelectionOverview;
  artefact?: BitloopsArtefact;
  embeddingRepresentations: EmbeddingRepresentationKind[];
}

export interface SelectionPreview extends Partial<SelectionDetails> {
  summary?: string | null;
}

export interface SelectionTarget {
  kind: SelectionKind;
  workspaceFolderFsPath: string;
  selector: ArtefactSelector;
  title: string;
  subtitle?: string;
  preview?: SelectionPreview;
}

export interface SidebarSearchResultItem {
  id: string;
  target: SelectionTarget;
  title: string;
  description: string;
  scoreLabel?: string;
  scoreBreakdownLabel?: string;
  matchBreakdownLabel?: string;
  summaryPreview?: string;
}

export interface SidebarSearchSection {
  id: string;
  title: string;
  description?: string;
  results: SidebarSearchResultItem[];
}

export interface SidebarStageChip {
  id: string;
  stage: StageKind;
  label: string;
  count: number;
  filterKey?: string;
  active: boolean;
  disabled: boolean;
}

export interface SidebarEmbeddingBadge {
  label: string;
  available: boolean;
}

export interface SidebarBreadcrumb {
  id: string;
  label: string;
  active: boolean;
}

export interface SidebarCheckpointFile {
  id: string;
  label: string;
  path: string;
  changeKind?: string;
}

export interface EditorNavigationTarget {
  path: string;
  startLine: number;
  endLine: number;
  symbolFqn?: string | null;
  canonicalKind?: string | null;
}

export interface SidebarCheckpointDetail {
  id: string;
  title: string;
  description?: string;
  metadata: string[];
  files: SidebarCheckpointFile[];
}

export interface SidebarStageRow {
  id: string;
  title: string;
  description?: string;
  detail?: string;
  navigationTarget?: EditorNavigationTarget;
  checkpointDetail?: SidebarCheckpointDetail;
}

export interface StageItemsResult {
  stage: StageKind;
  filterKey?: string;
  rows: SidebarStageRow[];
  emptyMessage: string;
}

export interface SidebarSelectionState {
  target: SelectionTarget;
  details: SelectionDetails;
}

export interface SidebarSelectionViewState {
  title: string;
  subtitle?: string;
  summary?: string;
  overviewTitle: string;
  overviewSegments: string[];
  badges: SidebarEmbeddingBadge[];
  chips: SidebarStageChip[];
  openInEditorLabel?: string;
}

export interface SidebarStageViewState {
  title: string;
  emptyMessage: string;
  rows: SidebarStageRow[];
}

export interface SidebarCheckpointViewState {
  id: string;
  title: string;
  description?: string;
  metadata: string[];
  files: SidebarCheckpointFile[];
}

export interface SidebarViewState {
  query: string;
  loading: boolean;
  loadingLabel?: string;
  statusMessage?: string;
  searchMode?: SearchMode;
  results: SidebarSearchResultItem[];
  searchSections: SidebarSearchSection[];
  totalCount: number;
  breadcrumbs: SidebarBreadcrumb[];
  selection?: SidebarSelectionViewState;
  stage?: SidebarStageViewState;
  checkpoint?: SidebarCheckpointViewState;
}

export interface OverviewCommandArgs {
  target: SelectionTarget;
}

export interface OpenSearchResultArgs {
  workspaceFolderFsPath: string;
  artefact: BitloopsArtefact;
}
