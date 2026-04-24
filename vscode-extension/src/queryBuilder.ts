import { ArtefactSelector } from './types';

export interface OverviewBatchAlias {
  alias: string;
  selector: ArtefactSelector;
}

export interface OverviewBatchQuery {
  aliases: OverviewBatchAlias[];
  query: string;
}

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

export function buildActiveFileQuery(relativePath: string, artefactLimit: number): string {
  return `{
  selectArtefacts(by: { path: ${gqlString(relativePath)} }) {
    count
    overview
    artefacts(first: ${clampPositiveInteger(artefactLimit)}) {
      path
      symbolFqn
      canonicalKind
      summary
      startLine
      endLine
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

export function buildSearchQuery(search: string, resultLimit: number): string {
  return `{
  selectArtefacts(by: { search: ${gqlString(search.trim())} }) {
    count
    artefacts(first: ${clampPositiveInteger(resultLimit)}) {
      path
      symbolFqn
      canonicalKind
      summary
      startLine
      endLine
    }
  }
}`;
}
