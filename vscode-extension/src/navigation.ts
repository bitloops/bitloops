import { BitloopsArtefact } from './types';

export interface ZeroBasedLineRange {
  startLine: number;
  endLine: number;
}

function normaliseCanonicalKind(kind?: string | null): string | undefined {
  if (!kind) {
    return undefined;
  }

  return kind.trim().toLowerCase().replace(/[_\s]+/g, '_');
}

export function canonicalKindLabel(kind?: string | null): string | undefined {
  const normalised = normaliseCanonicalKind(kind);
  if (!normalised) {
    return undefined;
  }

  return normalised.replace(/_/g, ' ');
}

export function canonicalKindIconId(kind?: string | null): string | undefined {
  const normalised = normaliseCanonicalKind(kind);
  if (!normalised) {
    return undefined;
  }

  switch (normalised) {
    case 'file':
      return 'file';
    case 'namespace':
    case 'module':
      return 'symbol-namespace';
    case 'import':
      return 'symbol-key';
    case 'type':
    case 'interface':
      return 'symbol-interface';
    case 'enum':
      return 'symbol-enum';
    case 'callable':
    case 'function':
      return 'symbol-function';
    case 'method':
      return 'symbol-method';
    case 'value':
    case 'variable':
    case 'member':
    case 'parameter':
    case 'type_parameter':
    case 'alias':
      return 'symbol-variable';
    default:
      return 'symbol-misc';
  }
}

export function toZeroBasedLineRange(
  artefact: Pick<BitloopsArtefact, 'startLine' | 'endLine'>,
): ZeroBasedLineRange {
  const startLine = Math.max(0, artefact.startLine - 1);
  const endLine = Math.max(startLine, artefact.endLine - 1);

  return {
    startLine,
    endLine,
  };
}

export function formatSearchResultDescription(artefact: BitloopsArtefact): string {
  const rangeText = `${artefact.path}:${artefact.startLine}-${artefact.endLine}`;
  const kind = canonicalKindLabel(artefact.canonicalKind);

  return kind ? `${rangeText} · ${kind}` : rangeText;
}
