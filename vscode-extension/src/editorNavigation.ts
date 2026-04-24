import * as path from 'node:path';

import * as vscode from 'vscode';

import { BitloopsArtefact, EditorNavigationTarget, SelectionTarget } from './types';
import { toZeroBasedLineRange } from './navigation';

function workspacePathUri(workspaceFolderFsPath: string, relativePath: string): vscode.Uri {
  return vscode.Uri.file(path.join(workspaceFolderFsPath, relativePath));
}

export async function openArtefactInEditor(
  workspaceFolderFsPath: string,
  artefact: Pick<BitloopsArtefact, 'path' | 'startLine' | 'endLine'>,
): Promise<void> {
  const fileUri = workspacePathUri(workspaceFolderFsPath, artefact.path);
  const { startLine, endLine } = toZeroBasedLineRange(artefact);
  const range = new vscode.Range(startLine, 0, endLine, 0);
  const editor = await vscode.window.showTextDocument(fileUri, {
    preview: false,
  });

  editor.selection = new vscode.Selection(range.start, range.end);
  editor.revealRange(range, vscode.TextEditorRevealType.InCenter);
}

export async function openNavigationTarget(
  workspaceFolderFsPath: string,
  target: EditorNavigationTarget,
): Promise<void> {
  await openArtefactInEditor(workspaceFolderFsPath, {
    path: target.path,
    startLine: target.startLine,
    endLine: target.endLine,
  });
}

export async function openSelectionTargetInEditor(target: SelectionTarget): Promise<void> {
  if (target.kind === 'file') {
    const document = await vscode.window.showTextDocument(
      workspacePathUri(target.workspaceFolderFsPath, target.selector.path),
      {
        preview: false,
      },
    );
    document.revealRange(new vscode.Range(0, 0, 0, 0), vscode.TextEditorRevealType.InCenter);
    return;
  }

  if (target.preview?.artefact) {
    await openArtefactInEditor(target.workspaceFolderFsPath, target.preview.artefact);
    return;
  }

  const lines = target.selector.lines ?? {
    start: 1,
    end: 1,
  };
  await openArtefactInEditor(target.workspaceFolderFsPath, {
    path: target.selector.path,
    startLine: lines.start,
    endLine: lines.end,
  });
}
