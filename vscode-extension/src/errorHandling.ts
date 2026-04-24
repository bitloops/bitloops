import * as vscode from 'vscode';

import { BitloopsCliError, OutputChannelLike } from './bitloopsCli';

export function toUserMessage(error: unknown, fallbackMessage: string): string {
  if (error instanceof BitloopsCliError) {
    return error.userMessage;
  }

  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }

  return fallbackMessage;
}

export function logError(
  outputChannel: OutputChannelLike | undefined,
  context: string,
  error: unknown,
): void {
  if (!outputChannel) {
    return;
  }

  const detail =
    error instanceof Error && error.stack ? error.stack : error instanceof Error ? error.message : String(error);
  outputChannel.appendLine(`[Bitloops] ${context}: ${detail}`);
}

export async function showBitloopsError(
  fallbackMessage: string,
  error: unknown,
  outputChannel?: OutputChannelLike,
): Promise<void> {
  logError(outputChannel, fallbackMessage, error);
  await vscode.window.showErrorMessage(toUserMessage(error, fallbackMessage));
}
