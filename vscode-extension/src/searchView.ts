import * as vscode from 'vscode';

import { openNavigationTarget, openSelectionTargetInEditor } from './editorNavigation';
import { showBitloopsError } from './errorHandling';
import { BitloopsOverviewService } from './overviewService';
import { BitloopsSearchService } from './searchService';
import { BitloopsSidebarController } from './sidebarController';
import { getBitloopsSettings } from './settings';
import { SelectionTarget, StageKind } from './types';
import { resolveActiveWorkspaceFolder } from './workspace';

const STAGE_RESULT_LIMIT = 20;

type SidebarMessage =
  | { type: 'ready' }
  | { type: 'search'; query?: string }
  | { type: 'selectResult'; resultId?: string }
  | { type: 'openStage'; stage?: StageKind; filterKey?: string }
  | { type: 'activateRow'; rowId?: string }
  | { type: 'breadcrumb'; id?: string }
  | { type: 'back' }
  | { type: 'refresh' }
  | { type: 'focusSearch' }
  | { type: 'openSelectionInEditor' }
  | { type: 'openCheckpointFile'; rowId?: string; fileId?: string };

function createNonce(): string {
  const alphabet = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
  let value = '';

  for (let index = 0; index < 24; index += 1) {
    value += alphabet[Math.floor(Math.random() * alphabet.length)];
  }

  return value;
}

export class BitloopsSearchView implements vscode.WebviewViewProvider {
  private readonly controller = new BitloopsSidebarController();
  private pendingFocusSearch = false;
  private view?: vscode.WebviewView;

  constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly searchService: BitloopsSearchService,
    private readonly overviewService: BitloopsOverviewService,
    private readonly outputChannel?: vscode.OutputChannel,
  ) {}

  async resolveWebviewView(
    webviewView: vscode.WebviewView,
    _context: vscode.WebviewViewResolveContext,
    _token: vscode.CancellationToken,
  ): Promise<void> {
    this.view = webviewView;
    webviewView.webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.extensionUri, 'media')],
    };
    webviewView.webview.html = this.renderHtml(webviewView.webview);

    webviewView.webview.onDidReceiveMessage((message: SidebarMessage) => {
      void this.handleMessage(message);
    });

    webviewView.onDidDispose(() => {
      if (this.view === webviewView) {
        this.view = undefined;
      }
    });

    this.postState();
    if (this.pendingFocusSearch) {
      this.pendingFocusSearch = false;
      void webviewView.webview.postMessage({
        type: 'focusSearch',
      });
    }
  }

  clear(): void {
    this.controller.clear();
    this.postState();
  }

  async focusSearch(): Promise<void> {
    await vscode.commands.executeCommand('workbench.view.extension.bitloopsSidebar');
    if (!this.view) {
      this.pendingFocusSearch = true;
      return;
    }

    this.view.show(false);
    void this.view.webview.postMessage({
      type: 'focusSearch',
    });
  }

  async revealSelection(
    target: SelectionTarget,
    interactive: boolean,
  ): Promise<void> {
    await this.focusSearch();
    this.controller.revealSelection(target);
    this.controller.beginLoading(
      target.kind === 'file' ? 'Loading file overview…' : 'Loading artefact details…',
    );
    this.postState();

    try {
      const details = await this.overviewService.loadSelectionDetails(
        target.workspaceFolderFsPath,
        target,
      );
      this.controller.applySelectionDetails(target, details);
      this.controller.setStatusMessage(undefined);
      this.postState();
    } catch (error) {
      this.controller.endLoading();
      this.controller.setStatusMessage('Bitloops could not load that selection.');
      this.postState();
      if (interactive) {
        await showBitloopsError(
          'Loading Bitloops selection details failed.',
          error,
          this.outputChannel,
        );
      }
    }
  }

  async refresh(): Promise<void> {
    const selection = this.controller.currentSelectionTarget();
    if (selection) {
      await this.revealSelection(selection, false);
      const activeStage = this.controller.currentStage();
      if (activeStage) {
        await this.loadStage(activeStage.stage, activeStage.filterKey, false);
      }
      return;
    }

    const query = this.controller.currentQuery();
    if (query.trim().length > 0) {
      const workspaceFolder = resolveActiveWorkspaceFolder();
      if (!workspaceFolder) {
        return;
      }

      await this.search(query, workspaceFolder, false);
    }
  }

  private async handleMessage(message: SidebarMessage): Promise<void> {
    switch (message.type) {
      case 'ready':
        this.postState();
        return;
      case 'focusSearch':
        await this.focusSearch();
        return;
      case 'search': {
        const query = message.query?.trim() ?? '';
        if (!query) {
          this.controller.clear();
          this.postState();
          return;
        }

        const workspaceFolder = resolveActiveWorkspaceFolder();
        if (!workspaceFolder) {
          await vscode.window.showErrorMessage(
            'Bitloops search requires an open workspace folder.',
          );
          return;
        }

        await this.search(query, workspaceFolder, true);
        return;
      }
      case 'selectResult': {
        const target =
          typeof message.resultId === 'string'
            ? this.controller.getSearchResultTarget(message.resultId)
            : undefined;
        if (!target) {
          return;
        }

        await this.revealSelection(target, true);
        return;
      }
      case 'openStage':
        if (!message.stage) {
          return;
        }
        await this.loadStage(message.stage, message.filterKey, true);
        return;
      case 'activateRow': {
        if (!message.rowId) {
          return;
        }

        const row = this.controller.getStageRow(message.rowId);
        if (!row) {
          return;
        }

        if (row.checkpointDetail) {
          this.controller.openCheckpointDetail(row.checkpointDetail);
          this.postState();
          return;
        }

        const selection = this.controller.currentSelectionTarget();
        if (!selection || !row.navigationTarget) {
          return;
        }

        try {
          await openNavigationTarget(selection.workspaceFolderFsPath, row.navigationTarget);
        } catch (error) {
          await showBitloopsError(
            'Opening Bitloops stage result failed.',
            error,
            this.outputChannel,
          );
        }
        return;
      }
      case 'openCheckpointFile': {
        if (!message.rowId || !message.fileId) {
          return;
        }

        const row = this.controller.getStageRow(message.rowId);
        const selection = this.controller.currentSelectionTarget();
        const checkpointFile = row?.checkpointDetail?.files.find((file) => file.id === message.fileId);
        if (!selection || !checkpointFile) {
          return;
        }

        try {
          await openNavigationTarget(selection.workspaceFolderFsPath, {
            path: checkpointFile.path,
            startLine: 1,
            endLine: 1,
          });
        } catch (error) {
          await showBitloopsError(
            'Opening Bitloops checkpoint file failed.',
            error,
            this.outputChannel,
          );
        }
        return;
      }
      case 'openSelectionInEditor': {
        const selection = this.controller.currentSelectionTarget();
        if (!selection) {
          return;
        }

        try {
          await openSelectionTargetInEditor(selection);
        } catch (error) {
          await showBitloopsError(
            'Opening Bitloops selection failed.',
            error,
            this.outputChannel,
          );
        }
        return;
      }
      case 'breadcrumb':
        if (!message.id) {
          return;
        }
        this.controller.navigateToBreadcrumb(message.id);
        this.postState();
        return;
      case 'back':
        this.controller.back();
        this.postState();
        return;
      case 'refresh':
        await this.refresh();
        return;
    }
  }

  private async search(
    query: string,
    workspaceFolder: vscode.WorkspaceFolder,
    interactive: boolean,
  ): Promise<void> {
    try {
      this.controller.beginLoading('Searching Bitloops artefacts…');
      this.postState();
      const settings = getBitloopsSettings();
      const result = await this.searchService.search(
        workspaceFolder.uri.fsPath,
        query,
        settings.searchResultLimit,
      );
      this.controller.setSearchResults(
        query,
        workspaceFolder.uri.fsPath,
        result.artefacts,
        result.count,
      );
      this.postState();
    } catch (error) {
      this.controller.endLoading();
      this.controller.setStatusMessage('Bitloops search failed.');
      this.postState();
      if (interactive) {
        await showBitloopsError('Bitloops search failed.', error, this.outputChannel);
      }
    }
  }

  private async loadStage(
    stage: StageKind,
    filterKey: string | undefined,
    interactive: boolean,
  ): Promise<void> {
    const selection = this.controller.currentSelectionTarget();
    if (!selection) {
      return;
    }

    try {
      this.controller.beginLoading('Loading related Bitloops results…');
      this.postState();
      const result = await this.overviewService.loadStageItems(
        selection.workspaceFolderFsPath,
        selection,
        stage,
        STAGE_RESULT_LIMIT,
        filterKey,
      );
      this.controller.setActiveStage(result);
      this.postState();
    } catch (error) {
      this.controller.endLoading();
      this.controller.setStatusMessage('Bitloops could not load related results.');
      this.postState();
      if (interactive) {
        await showBitloopsError(
          'Loading Bitloops stage items failed.',
          error,
          this.outputChannel,
        );
      }
    }
  }

  private postState(): void {
    if (!this.view) {
      return;
    }

    const state = this.controller.viewState();
    this.view.description =
      state.query.trim().length > 0 && state.totalCount > 0
        ? `${Math.min(state.results.length, state.totalCount)} / ${state.totalCount}`
        : undefined;
    void this.view.webview.postMessage({
      type: 'setState',
      state,
    });
  }

  private renderHtml(webview: vscode.Webview): string {
    const nonce = createNonce();
    const scriptUri = webview.asWebviewUri(
      vscode.Uri.joinPath(this.extensionUri, 'media', 'sidebar.js'),
    );
    const styleUri = webview.asWebviewUri(
      vscode.Uri.joinPath(this.extensionUri, 'media', 'sidebar.css'),
    );

    return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta
      http-equiv="Content-Security-Policy"
      content="default-src 'none'; img-src ${webview.cspSource} https:; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';"
    />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <link rel="stylesheet" href="${styleUri}" />
    <title>Bitloops</title>
  </head>
  <body>
    <div id="app" class="app"></div>
    <script nonce="${nonce}" src="${scriptUri}"></script>
  </body>
</html>`;
  }
}
