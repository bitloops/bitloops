import {
  buildEmbeddingBadges,
  formatOverviewCodeLensTitle,
  formatOverviewSegments,
  normaliseSummaryText,
} from './overviewFormatter';
import { canonicalKindLabel, formatSearchResultDescription } from './navigation';
import {
  BitloopsArtefact,
  SelectionDetails,
  SelectionKind,
  SelectionOverview,
  SelectionTarget,
  SidebarBreadcrumb,
  SidebarCheckpointDetail,
  SidebarSearchResultItem,
  SidebarSelectionState,
  SidebarStageChip,
  SidebarStageRow,
  SidebarViewState,
  StageItemsResult,
  StageKind,
} from './types';

interface ActiveStageState extends StageItemsResult {
  title: string;
}

interface SidebarInternalState {
  currentQuery: string;
  loading: boolean;
  loadingLabel?: string;
  statusMessage?: string;
  totalCount: number;
  results: SidebarSearchResultItem[];
  selection?: SidebarSelectionState;
  activeStage?: ActiveStageState;
  checkpoint?: SidebarCheckpointDetail;
}

function artefactTitle(artefact: Pick<BitloopsArtefact, 'path' | 'symbolFqn' | 'startLine' | 'endLine'>): string {
  if (artefact.symbolFqn && artefact.symbolFqn.trim().length > 0) {
    return artefact.symbolFqn;
  }

  return `${artefact.path}:${artefact.startLine}-${artefact.endLine}`;
}

function selectionDefaultTitle(
  target: SelectionTarget,
  details?: SelectionDetails,
): string {
  return details?.artefact ? artefactTitle(details.artefact) : target.title;
}

function selectionSubtitle(
  target: SelectionTarget,
  details?: SelectionDetails,
): string | undefined {
  if (details?.artefact) {
    return formatSearchResultDescription(details.artefact);
  }

  return target.subtitle;
}

function stageTitle(stage: StageKind, filterKey?: string): string {
  switch (stage) {
    case 'checkpoints':
      return 'Checkpoints';
    case 'dependencies':
      return filterKey ? `Dependencies · ${filterKey.replace(/_/g, ' ')}` : 'Dependencies';
    case 'codeMatches':
      return filterKey ? `Code matches · ${filterKey.replace(/_/g, ' ')}` : 'Code matches';
    case 'tests':
      return 'Tests';
  }
}

function buildBreadcrumbs(state: SidebarInternalState): SidebarBreadcrumb[] {
  const breadcrumbs: SidebarBreadcrumb[] = [
    {
      id: 'results',
      label: state.currentQuery.trim().length > 0 ? 'Search results' : 'Bitloops',
      active: !state.selection,
    },
  ];

  if (state.selection) {
    breadcrumbs.push({
      id: 'selection',
      label: state.selection.target.kind === 'file' ? 'File' : 'Artefact',
      active: !state.activeStage && !state.checkpoint,
    });
  }

  if (state.activeStage) {
    breadcrumbs.push({
      id: 'stage',
      label: state.activeStage.title,
      active: !state.checkpoint,
    });
  }

  if (state.checkpoint) {
    breadcrumbs.push({
      id: 'checkpoint',
      label: 'Checkpoint',
      active: true,
    });
  }

  return breadcrumbs;
}

function buildStageChips(
  overview: SelectionOverview,
  activeStage?: ActiveStageState,
): SidebarStageChip[] {
  const dependencyOverview = overview.dependencies?.overview?.dependencies;
  const dependencyKinds = dependencyOverview?.kindCounts ?? {};
  const codeMatchCounts = overview.codeMatches?.overview?.counts ?? {};
  const baseChips: SidebarStageChip[] = [
    {
      id: 'checkpoints',
      stage: 'checkpoints',
      label: 'Checkpoints',
      count: typeof overview.checkpoints?.overview?.totalCount === 'number'
        ? overview.checkpoints.overview.totalCount
        : 0,
      active: activeStage?.stage === 'checkpoints',
      disabled: !overview.checkpoints?.overview?.totalCount,
    },
    {
      id: 'dependencies',
      stage: 'dependencies',
      label: 'Dependencies',
      count: typeof dependencyOverview?.total === 'number' ? dependencyOverview.total : 0,
      active: activeStage?.stage === 'dependencies' && !activeStage.filterKey,
      disabled: !dependencyOverview?.total,
    },
    {
      id: 'codeMatches',
      stage: 'codeMatches',
      label: 'Code matches',
      count: typeof codeMatchCounts.total === 'number' ? codeMatchCounts.total : 0,
      active: activeStage?.stage === 'codeMatches' && !activeStage.filterKey,
      disabled: !codeMatchCounts.total,
    },
    {
      id: 'tests',
      stage: 'tests',
      label: 'Tests',
      count:
        typeof overview.tests?.overview?.totalCoveringTests === 'number'
          ? overview.tests.overview.totalCoveringTests
          : 0,
      active: activeStage?.stage === 'tests',
      disabled: !overview.tests?.overview?.totalCoveringTests,
    },
  ];

  for (const [kind, count] of Object.entries(dependencyKinds)) {
    if (typeof count !== 'number' || count <= 0) {
      continue;
    }

    baseChips.push({
      id: `dependencies:${kind}`,
      stage: 'dependencies',
      label: kind.replace(/_/g, ' '),
      count,
      filterKey: kind,
      active: activeStage?.stage === 'dependencies' && activeStage.filterKey === kind,
      disabled: false,
    });
  }

  for (const [kind, count] of Object.entries(codeMatchCounts)) {
    if (kind === 'total' || typeof count !== 'number' || count <= 0) {
      continue;
    }

    baseChips.push({
      id: `codeMatches:${kind}`,
      stage: 'codeMatches',
      label: kind.replace(/_/g, ' '),
      count,
      filterKey: kind,
      active: activeStage?.stage === 'codeMatches' && activeStage.filterKey === kind,
      disabled: false,
    });
  }

  return baseChips;
}

export class BitloopsSidebarController {
  private readonly resultLookup = new Map<string, SelectionTarget>();
  private readonly stageRowLookup = new Map<string, SidebarStageRow>();

  private state: SidebarInternalState = {
    currentQuery: '',
    loading: false,
    totalCount: 0,
    results: [],
  };

  clear(): void {
    this.resultLookup.clear();
    this.stageRowLookup.clear();
    this.state = {
      currentQuery: '',
      loading: false,
      totalCount: 0,
      results: [],
    };
  }

  beginLoading(label: string): void {
    this.state.loading = true;
    this.state.loadingLabel = label;
    this.state.statusMessage = undefined;
  }

  endLoading(): void {
    this.state.loading = false;
    this.state.loadingLabel = undefined;
  }

  setStatusMessage(message: string | undefined): void {
    this.state.statusMessage = message;
  }

  setSearchResults(
    query: string,
    workspaceFolderFsPath: string,
    artefacts: BitloopsArtefact[],
    totalCount: number,
  ): void {
    this.resultLookup.clear();
    this.state.currentQuery = query;
    this.state.totalCount = totalCount;
    this.state.results = artefacts.map((artefact, index) => {
      const id = `result-${index}`;
      const target: SelectionTarget = {
        kind: 'artefact',
        workspaceFolderFsPath,
        selector:
          artefact.symbolFqn && artefact.symbolFqn.trim().length > 0
            ? {
                path: artefact.path,
                symbolFqn: artefact.symbolFqn,
              }
            : {
                path: artefact.path,
                lines: {
                  start: artefact.startLine,
                  end: artefact.endLine,
                },
              },
        title: artefactTitle(artefact),
        subtitle: formatSearchResultDescription(artefact),
        preview: {
          artefact,
          summary: artefact.summary ?? undefined,
          embeddingRepresentations: artefact.embeddingRepresentations ?? [],
        },
      };

      this.resultLookup.set(id, target);
      return {
        id,
        target,
        title: target.title,
        description: target.subtitle ?? artefact.path,
        summaryPreview: normaliseSummaryText(artefact.summary),
      };
    });
    this.state.selection = undefined;
    this.state.activeStage = undefined;
    this.state.checkpoint = undefined;
    this.state.statusMessage =
      totalCount === 0
        ? `No artefacts found for “${query}”.`
        : undefined;
    this.endLoading();
  }

  getSearchResultTarget(id: string): SelectionTarget | undefined {
    return this.resultLookup.get(id);
  }

  currentQuery(): string {
    return this.state.currentQuery;
  }

  currentSelectionTarget(): SelectionTarget | undefined {
    return this.state.selection?.target;
  }

  currentStage():
    | {
        stage: StageKind;
        filterKey?: string;
      }
    | undefined {
    if (!this.state.activeStage) {
      return undefined;
    }

    return {
      stage: this.state.activeStage.stage,
      filterKey: this.state.activeStage.filterKey,
    };
  }

  revealSelection(target: SelectionTarget): void {
    const previewDetails = target.preview
      ? {
          count: target.preview.count ?? 0,
          overview: target.preview.overview ?? {},
          artefact: target.preview.artefact,
          embeddingRepresentations: target.preview.embeddingRepresentations ?? [],
        }
      : {
          count: 0,
          overview: {},
          artefact: undefined,
          embeddingRepresentations: [],
        };

    this.state.selection = {
      target,
      details: previewDetails,
    };
    this.state.activeStage = undefined;
    this.state.checkpoint = undefined;
  }

  applySelectionDetails(target: SelectionTarget, details: SelectionDetails): void {
    this.state.selection = {
      target,
      details,
    };
    this.state.activeStage = undefined;
    this.state.checkpoint = undefined;
    this.endLoading();
  }

  setActiveStage(result: StageItemsResult): void {
    this.stageRowLookup.clear();
    for (const row of result.rows) {
      this.stageRowLookup.set(row.id, row);
    }

    this.state.activeStage = {
      ...result,
      title: stageTitle(result.stage, result.filterKey),
    };
    this.state.checkpoint = undefined;
    this.endLoading();
  }

  getStageRow(id: string): SidebarStageRow | undefined {
    return this.stageRowLookup.get(id);
  }

  openCheckpointDetail(detail: SidebarCheckpointDetail): void {
    this.state.checkpoint = detail;
  }

  navigateToBreadcrumb(id: string): void {
    switch (id) {
      case 'results':
        this.state.selection = undefined;
        this.state.activeStage = undefined;
        this.state.checkpoint = undefined;
        break;
      case 'selection':
        this.state.activeStage = undefined;
        this.state.checkpoint = undefined;
        break;
      case 'stage':
        this.state.checkpoint = undefined;
        break;
      default:
        break;
    }
  }

  back(): void {
    if (this.state.checkpoint) {
      this.state.checkpoint = undefined;
      return;
    }

    if (this.state.activeStage) {
      this.state.activeStage = undefined;
      return;
    }

    if (this.state.selection) {
      this.state.selection = undefined;
    }
  }

  viewState(): SidebarViewState {
    const selection = this.state.selection;
    const selectionState = selection
      ? {
          title: selectionDefaultTitle(selection.target, selection.details),
          subtitle: selectionSubtitle(selection.target, selection.details),
          summary: normaliseSummaryText(selection.details.artefact?.summary),
          overviewTitle: formatOverviewCodeLensTitle(selection.details.overview),
          overviewSegments: formatOverviewSegments(selection.details.overview),
          badges: buildEmbeddingBadges(selection.details.embeddingRepresentations),
          chips: buildStageChips(selection.details.overview, this.state.activeStage),
          openInEditorLabel:
            selection.target.kind === 'file'
              ? selection.target.selector.path
              : selection.details.artefact?.path ?? selection.target.selector.path,
        }
      : undefined;

    return {
      query: this.state.currentQuery,
      loading: this.state.loading,
      loadingLabel: this.state.loadingLabel,
      statusMessage: this.state.statusMessage,
      results: this.state.results,
      totalCount: this.state.totalCount,
      breadcrumbs: buildBreadcrumbs(this.state),
      selection: selectionState,
      stage: this.state.activeStage
        ? {
            title: this.state.activeStage.title,
            emptyMessage: this.state.activeStage.emptyMessage,
            rows: this.state.activeStage.rows,
          }
        : undefined,
      checkpoint: this.state.checkpoint
        ? {
            id: this.state.checkpoint.id,
            title: this.state.checkpoint.title,
            description: this.state.checkpoint.description,
            metadata: this.state.checkpoint.metadata,
            files: this.state.checkpoint.files,
          }
        : undefined,
    };
  }

  static createSelectionTarget(
    workspaceFolderFsPath: string,
    kind: SelectionKind,
    selector: SelectionTarget['selector'],
    title: string,
    subtitle?: string,
    preview?: SelectionTarget['preview'],
  ): SelectionTarget {
    return {
      kind,
      workspaceFolderFsPath,
      selector,
      title,
      subtitle,
      preview,
    };
  }

  static describeArtefact(artefact: BitloopsArtefact): string {
    const kind = canonicalKindLabel(artefact.canonicalKind);
    const pathAndRange = `${artefact.path}:${artefact.startLine}-${artefact.endLine}`;
    return kind ? `${pathAndRange} · ${kind}` : pathAndRange;
  }
}
