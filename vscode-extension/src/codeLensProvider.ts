import * as vscode from 'vscode';

import { showBitloopsError } from './errorHandling';
import { formatOverviewCodeLensTitle, formatSummaryCodeLensTitle } from './overviewFormatter';
import { BitloopsOverviewService } from './overviewService';
import { getBitloopsSettings } from './settings';
import { DocumentOverviewData, OverviewCommandArgs } from './types';
import { resolveDocumentWorkspaceFolder, toRelativeWorkspacePath } from './workspace';

function artefactTitle(data: {
  path: string;
  symbolFqn?: string | null;
  startLine: number;
  endLine: number;
}): string {
  if (data.symbolFqn && data.symbolFqn.trim().length > 0) {
    return data.symbolFqn;
  }

  return `${data.path}:${data.startLine}-${data.endLine}`;
}

export class BitloopsOverviewCodeLensProvider implements vscode.CodeLensProvider {
  private readonly didChangeCodeLenses = new vscode.EventEmitter<void>();
  private readonly cache = new Map<string, DocumentOverviewData>();
  private readonly inFlight = new Set<string>();
  private readonly requestVersions = new Map<string, number>();

  constructor(
    private readonly overviewService: BitloopsOverviewService,
    private readonly outputChannel?: vscode.OutputChannel,
  ) {}

  readonly onDidChangeCodeLenses = this.didChangeCodeLenses.event;

  clearAll(): void {
    this.cache.clear();
    this.inFlight.clear();
    this.didChangeCodeLenses.fire();
  }

  async refreshActiveDocument(interactive = false): Promise<void> {
    const document = vscode.window.activeTextEditor?.document;
    if (!document) {
      return;
    }

    await this.refreshDocument(document, interactive);
  }

  async refreshDocument(
    document: vscode.TextDocument,
    interactive = false,
  ): Promise<void> {
    if (document.uri.scheme !== 'file') {
      return;
    }

    const workspaceFolder = resolveDocumentWorkspaceFolder(document);
    const relativePath =
      workspaceFolder && toRelativeWorkspacePath(workspaceFolder, document.uri);
    const cacheKey = document.uri.toString();

    if (!workspaceFolder || !relativePath) {
      this.cache.delete(cacheKey);
      this.didChangeCodeLenses.fire();
      return;
    }

    const requestVersion = (this.requestVersions.get(cacheKey) ?? 0) + 1;
    this.requestVersions.set(cacheKey, requestVersion);
    this.inFlight.add(cacheKey);

    try {
      const settings = getBitloopsSettings();
      const data = await this.overviewService.loadFileOverview(
        workspaceFolder.uri.fsPath,
        relativePath,
        settings.activeFileArtefactLimit,
      );

      if (this.requestVersions.get(cacheKey) !== requestVersion) {
        return;
      }

      this.cache.set(cacheKey, data);
      this.didChangeCodeLenses.fire();
    } catch (error) {
      if (this.requestVersions.get(cacheKey) !== requestVersion) {
        return;
      }

      this.cache.delete(cacheKey);
      this.didChangeCodeLenses.fire();
      if (interactive) {
        await showBitloopsError(
          'Refreshing Bitloops overview failed.',
          error,
          this.outputChannel,
        );
      }
    } finally {
      this.inFlight.delete(cacheKey);
    }
  }

  provideCodeLenses(
    document: vscode.TextDocument,
    _token: vscode.CancellationToken,
  ): vscode.ProviderResult<vscode.CodeLens[]> {
    const cacheKey = document.uri.toString();
    const cached = this.cache.get(cacheKey);

    if (!cached) {
      if (getBitloopsSettings().autoRefresh && !this.inFlight.has(cacheKey)) {
        void this.refreshDocument(document, false);
      }

      return [];
    }

    return this.buildCodeLenses(cached);
  }

  private buildCodeLenses(data: DocumentOverviewData): vscode.CodeLens[] {
    const lenses: vscode.CodeLens[] = [];
    const fileCommandArgs: OverviewCommandArgs = {
      title: `Bitloops file overview · ${data.path}`,
      overview: data.overview,
    };

    lenses.push(
      new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), {
        title: formatOverviewCodeLensTitle(data.overview),
        tooltip: fileCommandArgs.title,
        command: 'bitloops.showOverviewDetails',
        arguments: [fileCommandArgs],
      }),
    );

    for (const artefact of data.artefacts) {
      const summaryTitle = formatSummaryCodeLensTitle(artefact.summary);
      if (!summaryTitle && !artefact.overview) {
        continue;
      }

      const line = Math.max(0, artefact.startLine - 1);
      const commandArgs: OverviewCommandArgs = {
        title: `Bitloops artefact details · ${artefactTitle(artefact)}`,
        overview: artefact.overview ?? {},
        summary: artefact.summary,
      };

      if (summaryTitle) {
        lenses.push(
          new vscode.CodeLens(new vscode.Range(line, 0, line, 0), {
            title: summaryTitle,
            tooltip: commandArgs.title,
            command: 'bitloops.showOverviewDetails',
            arguments: [commandArgs],
          }),
        );
      }

      if (artefact.overview) {
        lenses.push(
          new vscode.CodeLens(new vscode.Range(line, 0, line, 0), {
            title: formatOverviewCodeLensTitle(artefact.overview),
            tooltip: commandArgs.title,
            command: 'bitloops.showOverviewDetails',
            arguments: [commandArgs],
          }),
        );
      }
    }

    return lenses;
  }
}
