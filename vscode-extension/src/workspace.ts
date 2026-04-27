import * as path from 'node:path';

import * as vscode from 'vscode';

export function resolveDocumentWorkspaceFolder(
  document: vscode.TextDocument,
): vscode.WorkspaceFolder | undefined {
  return vscode.workspace.getWorkspaceFolder(document.uri);
}

export function resolveActiveWorkspaceFolder(): vscode.WorkspaceFolder | undefined {
  const activeDocument = vscode.window.activeTextEditor?.document;
  if (activeDocument) {
    const activeFolder = resolveDocumentWorkspaceFolder(activeDocument);
    if (activeFolder) {
      return activeFolder;
    }
  }

  const folders = vscode.workspace.workspaceFolders ?? [];
  return folders.length > 0 ? folders[0] : undefined;
}

export function toRelativeWorkspacePath(
  workspaceFolder: vscode.WorkspaceFolder,
  resource: vscode.Uri,
): string | undefined {
  const relativePath = path.relative(workspaceFolder.uri.fsPath, resource.fsPath);
  if (!relativePath || relativePath.startsWith('..') || path.isAbsolute(relativePath)) {
    return undefined;
  }

  return relativePath.split(path.sep).join('/');
}
