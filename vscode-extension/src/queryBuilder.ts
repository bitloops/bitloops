import { ArtefactSelector, StageKind } from './types';

export interface OverviewBatchAlias {
  alias: string;
  selector: ArtefactSelector;
}

export interface OverviewBatchQuery {
  aliases: OverviewBatchAlias[];
  query: string;
}

export interface ArtefactFieldSupport {
  summary: boolean;
  embeddingRepresentations: boolean;
}

export interface StageItemsRequest {
  stage: StageKind;
  filterKey?: string;
  resultLimit: number;
}

export const DEFAULT_ARTEFACT_FIELD_SUPPORT: ArtefactFieldSupport = {
  summary: true,
  embeddingRepresentations: true,
};

function clampPositiveInteger(value: number): number {
  if (!Number.isFinite(value) || value <= 0) {
    return 1;
  }

  return Math.floor(value);
}

function gqlString(value: string): string {
  return JSON.stringify(value);
}

function selectorLiteral(selector: ArtefactSelector): string {
  const parts: string[] = [`path: ${gqlString(selector.path)}`];

  if (selector.symbolFqn && selector.symbolFqn.trim().length > 0) {
    return `{
      symbolFqn: ${gqlString(selector.symbolFqn.trim())}
    }`;
  }

  if (selector.lines) {
    parts.push(`lines: { start: ${selector.lines.start}, end: ${selector.lines.end} }`);
  }

  return `{ ${parts.join(', ')} }`;
}

function artefactFieldList(fieldSupport: ArtefactFieldSupport): string[] {
  const fields = ['path', 'symbolFqn', 'canonicalKind'];

  if (fieldSupport.summary) {
    fields.push('summary');
  }

  if (fieldSupport.embeddingRepresentations) {
    fields.push('embeddingRepresentations');
  }

  fields.push('startLine', 'endLine');
  return fields;
}

function nestedArtefactFields(): string {
  return `{
        path
        symbolFqn
        canonicalKind
        startLine
        endLine
      }`;
}

function dependencyKindArgument(filterKey?: string): string {
  if (!filterKey) {
    return '';
  }

  return `, kind: ${filterKey.trim().toUpperCase()}`;
}

function relationKindArgument(filterKey?: string): string {
  if (!filterKey) {
    return '';
  }

  return `relationKind: ${gqlString(filterKey.trim())}`;
}

function optionalArgumentList(argument: string): string {
  return argument.trim().length > 0 ? `(${argument})` : '';
}

export function buildActiveFileQuery(
  relativePath: string,
  artefactLimit: number,
  fieldSupport: ArtefactFieldSupport = DEFAULT_ARTEFACT_FIELD_SUPPORT,
): string {
  return `{
  selectArtefacts(by: { path: ${gqlString(relativePath)} }) {
    count
    overview
    artefacts(first: ${clampPositiveInteger(artefactLimit)}) {
      ${artefactFieldList(fieldSupport).join('\n      ')}
    }
  }
}`;
}

export function buildSelectionOverviewQuery(selector: ArtefactSelector): string {
  return `{
  selectArtefacts(by: ${selectorLiteral(selector)}) {
    count
    overview
  }
}`;
}

export function buildSelectionDetailsQuery(
  selector: ArtefactSelector,
  fieldSupport: ArtefactFieldSupport = DEFAULT_ARTEFACT_FIELD_SUPPORT,
): string {
  return `{
  selectArtefacts(by: ${selectorLiteral(selector)}) {
    count
    overview
    artefacts(first: 1) {
      ${artefactFieldList(fieldSupport).join('\n      ')}
    }
  }
}`;
}

export function buildArtefactOverviewBatchQuery(
  selectors: ArtefactSelector[],
): OverviewBatchQuery | undefined {
  if (selectors.length === 0) {
    return undefined;
  }

  const aliases = selectors.map((selector, index) => ({
    alias: `artefact${index}`,
    selector,
  }));

  const selections = aliases
    .map(
      ({ alias, selector }) =>
        `  ${alias}: selectArtefacts(by: ${selectorLiteral(selector)}) {\n    overview\n  }`,
    )
    .join('\n');

  return {
    aliases,
    query: `{\n${selections}\n}`,
  };
}

export function buildStageItemsQuery(
  selector: ArtefactSelector,
  request: StageItemsRequest,
): string {
  const resultLimit = clampPositiveInteger(request.resultLimit);
  let stageSelection: string;

  switch (request.stage) {
    case 'dependencies':
      stageSelection = `dependencies(direction: BOTH, includeUnresolved: true${dependencyKindArgument(
        request.filterKey,
      )}) {
      items(first: ${resultLimit}) {
        id
        edgeKind
        startLine
        endLine
        toSymbolRef
        fromArtefact ${nestedArtefactFields()}
        toArtefact ${nestedArtefactFields()}
      }
    }`;
      break;
    case 'codeMatches':
      stageSelection = `codeMatches${optionalArgumentList(relationKindArgument(request.filterKey))} {
      items(first: ${resultLimit}) {
        id
        relationKind
        score
        sourceStartLine
        sourceEndLine
        targetStartLine
        targetEndLine
        sourceArtefact ${nestedArtefactFields()}
        targetArtefact ${nestedArtefactFields()}
      }
    }`;
      break;
    case 'tests':
      stageSelection = `tests {
      items(first: ${resultLimit}) {
        artefact {
          artefactId
          name
          kind
          filePath
          startLine
          endLine
        }
        coveringTests {
          testId
          testName
          suiteName
          filePath
          startLine
          endLine
          confidence
          discoverySource
          linkageSource
          linkageStatus
          classification
        }
        summary {
          totalCoveringTests
          crossCutting
          dataSources
          diagnosticCount
        }
      }
    }`;
      break;
    case 'checkpoints':
      stageSelection = `checkpoints {
      items(first: ${resultLimit}) {
        id
        sessionId
        commitSha
        branch
        agent
        eventTime
        strategy
        filesTouched
        firstPromptPreview
        createdAt
        fileRelations {
          filepath
          changeKind
          pathBefore
          pathAfter
        }
      }
    }`;
      break;
  }

  return `{
  selectArtefacts(by: ${selectorLiteral(selector)}) {
    ${stageSelection}
  }
}`;
}

export function buildSearchQuery(
  search: string,
  resultLimit: number,
  fieldSupport: ArtefactFieldSupport = DEFAULT_ARTEFACT_FIELD_SUPPORT,
): string {
  return `{
  selectArtefacts(by: { search: ${gqlString(search.trim())} }) {
    count
    artefacts(first: ${clampPositiveInteger(resultLimit)}) {
      ${artefactFieldList(fieldSupport).join('\n      ')}
    }
  }
}`;
}
