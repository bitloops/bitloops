import * as vscode from 'vscode';

import { BitloopsSearchService } from './searchService';
import { showBitloopsError } from './errorHandling';
import { canonicalKindIconId, formatSearchResultDescription } from './navigation';
import { getBitloopsSettings } from './settings';
import { BitloopsArtefact, OpenSearchResultArgs } from './types';
import { resolveActiveWorkspaceFolder } from './workspace';

interface SearchTreeNode {
  artefact: BitloopsArtefact;
  workspaceFolderFsPath: string;
}

export class BitloopsSearchView implements vscode.TreeDataProvider<SearchTreeNode> {
  private readonly didChangeTreeData = new vscode.EventEmitter<SearchTreeNode | undefined | void>();

  private currentQuery: string | undefined;
  private results: SearchTreeNode[] = [];
  private totalCount = 0;
  private treeView?: vscode.TreeView<SearchTreeNode>;

  constructor(
    private readonly searchService: BitloopsSearchService,
    private readonly outputChannel?: vscode.OutputChannel,
  ) {}

  readonly onDidChangeTreeData = this.didChangeTreeData.event;

  attachTreeView(treeView: vscode.TreeView<SearchTreeNode>): void {
    this.treeView = treeView;
    this.updateTreeMessage();
  }

  clear(): void {
    this.currentQuery = undefined;
    this.results = [];
    this.totalCount = 0;
    this.updateTreeMessage();
    this.didChangeTreeData.fire();
  }

  async promptAndSearch(): Promise<void> {
    const query = await vscode.window.showInputBox({
      ignoreFocusOut: true,
      placeHolder: 'Search artefacts with Bitloops DevQL',
      prompt: 'Enter plain text to search semantic and fuzzy artefact matches.',
      value: this.currentQuery ?? '',
    });

    if (query === undefined) {
      return;
    }

    const trimmed = query.trim();
    if (!trimmed) {
      this.clear();
      return;
    }

    const workspaceFolder = resolveActiveWorkspaceFolder();
    if (!workspaceFolder) {
      await vscode.window.showErrorMessage(
        'Bitloops search requires an open workspace folder.',
      );
      return;
    }

    await this.search(trimmed, workspaceFolder, true);
  }

  async search(
    query: string,
    workspaceFolder: vscode.WorkspaceFolder,
    interactive: boolean,
  ): Promise<void> {
    const settings = getBitloopsSettings();

    try {
      const result = await this.searchService.search(
        workspaceFolder.uri.fsPath,
        query,
        settings.searchResultLimit,
      );

      this.currentQuery = query;
      this.totalCount = result.count;
      this.results = result.artefacts.map((artefact) => ({
        artefact,
        workspaceFolderFsPath: workspaceFolder.uri.fsPath,
      }));
      this.updateTreeMessage();
      this.didChangeTreeData.fire();
    } catch (error) {
      if (interactive) {
        await showBitloopsError('Bitloops search failed.', error, this.outputChannel);
      }
    }
  }

  getTreeItem(element: SearchTreeNode): vscode.TreeItem {
    const label =
      element.artefact.symbolFqn && element.artefact.symbolFqn.trim().length > 0
        ? element.artefact.symbolFqn
        : `${element.artefact.path}:${element.artefact.startLine}-${element.artefact.endLine}`;
    const treeItem = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    const iconId = canonicalKindIconId(element.artefact.canonicalKind);
    const description = formatSearchResultDescription(element.artefact);

    treeItem.description = description;
    treeItem.tooltip = element.artefact.summary?.trim() || description;
    treeItem.iconPath = iconId ? new vscode.ThemeIcon(iconId) : undefined;
    treeItem.command = {
      title: 'Open Bitloops search result',
      command: 'bitloops.openSearchResult',
      arguments: [
        {
          workspaceFolderFsPath: element.workspaceFolderFsPath,
          artefact: element.artefact,
        } satisfies OpenSearchResultArgs,
      ],
    };

    return treeItem;
  }

  getChildren(element?: SearchTreeNode): vscode.ProviderResult<SearchTreeNode[]> {
    if (element) {
      return [];
    }

    return this.results;
  }

  private updateTreeMessage(): void {
    if (!this.treeView) {
      return;
    }

    if (!this.currentQuery) {
      this.treeView.message = 'Run “Bitloops: Search Artefacts” to search the active workspace folder.';
      return;
    }

    if (this.totalCount === 0) {
      this.treeView.message = `No artefacts found for “${this.currentQuery}”.`;
      return;
    }

    const showing =
      this.results.length < this.totalCount
        ? `Showing ${this.results.length} of ${this.totalCount}`
        : `Showing ${this.totalCount}`;
    this.treeView.message = `${showing} results for “${this.currentQuery}”.`;
  }
}
