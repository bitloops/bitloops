import * as vscode from 'vscode';

export interface BitloopsSettings {
  cliPath: string;
  autoRefresh: boolean;
  searchResultLimit: number;
  activeFileArtefactLimit: number;
}

export function getBitloopsSettings(): BitloopsSettings {
  const config = vscode.workspace.getConfiguration('bitloops');

  return {
    cliPath: config.get<string>('cliPath', 'bitloops'),
    autoRefresh: config.get<boolean>('autoRefresh', true),
    searchResultLimit: config.get<number>('searchResultLimit', 20),
    activeFileArtefactLimit: config.get<number>('activeFileArtefactLimit', 200),
  };
}
