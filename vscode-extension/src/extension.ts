import * as path from 'node:path';

import * as vscode from 'vscode';

import { BitloopsCliClient } from './bitloopsCli';
import { BitloopsOverviewCodeLensProvider } from './codeLensProvider';
import { showBitloopsError } from './errorHandling';
import { canonicalKindIconId, formatSearchResultDescription, toZeroBasedLineRange } from './navigation';
import { formatOverviewDetailRows } from './overviewFormatter';
import { BitloopsOverviewService } from './overviewService';
import { BitloopsSearchService } from './searchService';
import { BitloopsSearchView } from './searchView';
import { getBitloopsSettings } from './settings';
import { OpenSearchResultArgs, OverviewCommandArgs } from './types';

export async function activate(
  context: vscode.ExtensionContext,
): Promise<{ searchView: BitloopsSearchView }> {
  const outputChannel = vscode.window.createOutputChannel('Bitloops');
  const cliClient = new BitloopsCliClient({
    getCliPath: () => getBitloopsSettings().cliPath,
    outputChannel,
  });
  const overviewService = new BitloopsOverviewService(cliClient);
  const searchService = new BitloopsSearchService(cliClient);
  const codeLensProvider = new BitloopsOverviewCodeLensProvider(overviewService, outputChannel);
  const searchView = new BitloopsSearchView(searchService, outputChannel);
  const treeView = vscode.window.createTreeView('bitloopsSearchView', {
    treeDataProvider: searchView,
    showCollapseAll: false,
  });

  searchView.attachTreeView(treeView);

  context.subscriptions.push(
    outputChannel,
    treeView,
    vscode.languages.registerCodeLensProvider({ scheme: 'file' }, codeLensProvider),
    vscode.commands.registerCommand('bitloops.searchArtefacts', async () => {
      await searchView.promptAndSearch();
    }),
    vscode.commands.registerCommand(
      'bitloops.refreshActiveFileOverview',
      async () => {
        await codeLensProvider.refreshActiveDocument(true);
      },
    ),
    vscode.commands.registerCommand(
      'bitloops.showOverviewDetails',
      async (args: OverviewCommandArgs | undefined) => {
        if (!args) {
          return;
        }

        const items = formatOverviewDetailRows(args.overview, args.summary).map((item) => ({
          label: item.label,
          description: item.description,
        }));

        await vscode.window.showQuickPick(items, {
          ignoreFocusOut: true,
          placeHolder: args.title,
        });
      },
    ),
    vscode.commands.registerCommand(
      'bitloops.openSearchResult',
      async (args: OpenSearchResultArgs | undefined) => {
        if (!args) {
          return;
        }

        try {
          const fileUri = vscode.Uri.file(
            path.join(args.workspaceFolderFsPath, args.artefact.path),
          );
          const { startLine, endLine } = toZeroBasedLineRange(args.artefact);
          const range = new vscode.Range(startLine, 0, endLine, 0);
          const editor = await vscode.window.showTextDocument(fileUri, {
            preview: false,
          });

          editor.selection = new vscode.Selection(range.start, range.end);
          editor.revealRange(range, vscode.TextEditorRevealType.InCenter);
        } catch (error) {
          await showBitloopsError('Opening Bitloops search result failed.', error, outputChannel);
        }
      },
    ),
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (!editor || !getBitloopsSettings().autoRefresh) {
        return;
      }

      void codeLensProvider.refreshDocument(editor.document, false);
    }),
    vscode.workspace.onDidSaveTextDocument((document) => {
      if (!getBitloopsSettings().autoRefresh) {
        return;
      }

      void codeLensProvider.refreshDocument(document, false);
    }),
    vscode.workspace.onDidChangeWorkspaceFolders(() => {
      codeLensProvider.clearAll();
      searchView.clear();
      if (!getBitloopsSettings().autoRefresh) {
        return;
      }

      void codeLensProvider.refreshActiveDocument(false);
    }),
  );

  if (getBitloopsSettings().autoRefresh) {
    void codeLensProvider.refreshActiveDocument(false);
  }

  return {
    searchView,
  };
}

export function deactivate(): void {}
