import * as vscode from 'vscode';

import { BitloopsCliClient } from './bitloopsCli';
import { BitloopsOverviewCodeLensProvider } from './codeLensProvider';
import { openArtefactInEditor } from './editorNavigation';
import { showBitloopsError } from './errorHandling';
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
  const searchView = new BitloopsSearchView(
    context.extensionUri,
    searchService,
    overviewService,
    outputChannel,
  );

  context.subscriptions.push(
    outputChannel,
    vscode.window.registerWebviewViewProvider('bitloopsSearchView', searchView, {
      webviewOptions: {
        retainContextWhenHidden: true,
      },
    }),
    vscode.languages.registerCodeLensProvider({ scheme: 'file' }, codeLensProvider),
    vscode.commands.registerCommand('bitloops.searchArtefacts', async () => {
      await searchView.focusSearch();
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

        await searchView.revealSelection(args.target, true);
      },
    ),
    vscode.commands.registerCommand(
      'bitloops.openSearchResult',
      async (args: OpenSearchResultArgs | undefined) => {
        if (!args) {
          return;
        }

        try {
          await openArtefactInEditor(args.workspaceFolderFsPath, args.artefact);
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
