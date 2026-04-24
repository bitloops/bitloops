import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

export interface DaemonStatusReport {
  runtime?: {
    url?: string;
  } | null;
}

export interface CommandExecutionResult {
  stderr: string;
  stdout: string;
}

export interface CommandExecutor {
  execute(command: string, args: string[], options: { cwd: string }): Promise<CommandExecutionResult>;
}

export interface OutputChannelLike {
  appendLine(value: string): void;
}

export interface BitloopsQueryClient {
  executeGraphqlQuery<T>(cwd: string, query: string): Promise<T>;
}

export interface BitloopsCliClientOptions {
  daemonStatusTtlMs?: number;
  executor?: CommandExecutor;
  getCliPath?: () => string;
  now?: () => number;
  outputChannel?: OutputChannelLike;
}

export class CommandExecutionError extends Error {
  constructor(
    message: string,
    readonly stderr = '',
    readonly stdout = '',
    readonly exitCode?: number,
    readonly code?: string,
  ) {
    super(message);
    this.name = 'CommandExecutionError';
  }
}

export class BitloopsCliError extends Error {
  constructor(
    message: string,
    readonly userMessage: string,
    readonly detail?: string,
  ) {
    super(message);
    this.name = 'BitloopsCliError';
  }
}

export function errorMentionsUnknownField(error: unknown, fieldName: string): boolean {
  if (!(error instanceof BitloopsCliError)) {
    return false;
  }

  const haystack = [error.message, error.userMessage, error.detail ?? ''].join('\n');
  return haystack.includes(`Unknown field "${fieldName}"`);
}

class NodeCommandExecutor implements CommandExecutor {
  async execute(
    command: string,
    args: string[],
    options: { cwd: string },
  ): Promise<CommandExecutionResult> {
    try {
      const result = await execFileAsync(command, args, {
        cwd: options.cwd,
        maxBuffer: 10 * 1024 * 1024,
      });

      return {
        stdout: result.stdout,
        stderr: result.stderr,
      };
    } catch (error) {
      const failed = error as NodeJS.ErrnoException & {
        code?: string | number;
        stderr?: string;
        stdout?: string;
      };

      throw new CommandExecutionError(
        `Command failed: ${command} ${args.join(' ')}`.trim(),
        failed.stderr ?? '',
        failed.stdout ?? '',
        typeof failed.code === 'number' ? failed.code : undefined,
        typeof failed.code === 'string' ? failed.code : undefined,
      );
    }
  }
}

function isCommandExecutionError(error: unknown): error is CommandExecutionError {
  return error instanceof CommandExecutionError;
}

function parseJson<T>(raw: string, userMessage: string): T {
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    throw new BitloopsCliError(
      'Bitloops returned invalid JSON.',
      userMessage,
      error instanceof Error ? error.message : String(error),
    );
  }
}

function nonEmpty(value: string): string {
  return value.trim();
}

export class BitloopsCliClient implements BitloopsQueryClient {
  private readonly daemonStatusCache = new Map<
    string,
    {
      expiresAt: number;
      report: DaemonStatusReport;
    }
  >();

  private readonly daemonStatusTtlMs: number;
  private readonly executor: CommandExecutor;
  private readonly getCliPath: () => string;
  private readonly now: () => number;
  private readonly outputChannel?: OutputChannelLike;

  constructor(options: BitloopsCliClientOptions = {}) {
    this.daemonStatusTtlMs = options.daemonStatusTtlMs ?? 5_000;
    this.executor = options.executor ?? new NodeCommandExecutor();
    this.getCliPath = options.getCliPath ?? (() => 'bitloops');
    this.now = options.now ?? (() => Date.now());
    this.outputChannel = options.outputChannel;
  }

  async ensureDaemonAvailable(cwd: string): Promise<DaemonStatusReport> {
    const cached = this.daemonStatusCache.get(cwd);
    if (cached && cached.expiresAt > this.now()) {
      return cached.report;
    }

    const report = await this.runDaemonStatus(cwd);
    this.daemonStatusCache.set(cwd, {
      report,
      expiresAt: this.now() + this.daemonStatusTtlMs,
    });

    return report;
  }

  async executeGraphqlQuery<T>(cwd: string, query: string): Promise<T> {
    await this.ensureDaemonAvailable(cwd);
    const cliPath = this.getCliPath();
    const args = ['devql', 'query', '--graphql', '--compact', query];

    this.outputChannel?.appendLine(`[Bitloops] ${cliPath} ${args.join(' ')}`);

    try {
      const result = await this.executor.execute(cliPath, args, { cwd });
      const stdout = nonEmpty(result.stdout);
      if (!stdout) {
        throw new BitloopsCliError(
          'Bitloops query returned no output.',
          'Bitloops query returned no JSON output.',
          nonEmpty(result.stderr),
        );
      }

      return parseJson<T>(stdout, 'Bitloops query returned invalid JSON.');
    } catch (error) {
      throw this.mapCommandError(
        error,
        'Bitloops query failed. Run `bitloops status` to verify the daemon and workspace scope.',
      );
    }
  }

  private async runDaemonStatus(cwd: string): Promise<DaemonStatusReport> {
    const cliPath = this.getCliPath();
    const args = ['daemon', 'status', '--json'];

    this.outputChannel?.appendLine(`[Bitloops] ${cliPath} ${args.join(' ')}`);

    try {
      const result = await this.executor.execute(cliPath, args, { cwd });
      const stdout = nonEmpty(result.stdout);
      if (!stdout) {
        throw new BitloopsCliError(
          'Bitloops daemon status returned no output.',
          'Bitloops daemon status returned no JSON output.',
          nonEmpty(result.stderr),
        );
      }

      const report = parseJson<DaemonStatusReport>(
        stdout,
        'Bitloops daemon status returned invalid JSON.',
      );

      if (!report.runtime?.url) {
        throw new BitloopsCliError(
          'Bitloops daemon is not running.',
          'Bitloops daemon is not running for this workspace. Run `bitloops status`.',
        );
      }

      return report;
    } catch (error) {
      throw this.mapCommandError(
        error,
        'Bitloops daemon is unavailable. Run `bitloops status`.',
      );
    }
  }

  private mapCommandError(error: unknown, fallbackMessage: string): BitloopsCliError {
    if (error instanceof BitloopsCliError) {
      return error;
    }

    if (isCommandExecutionError(error)) {
      if (error.code === 'ENOENT') {
        const cliPath = this.getCliPath();
        return new BitloopsCliError(
          'Bitloops CLI was not found.',
          `Bitloops CLI was not found at \`${cliPath}\`. Install \`bitloops\` or update \`bitloops.cliPath\`.`,
        );
      }

      const detail = [error.stderr, error.stdout].filter(Boolean).join('\n').trim();
      return new BitloopsCliError(error.message, detail || fallbackMessage, detail || undefined);
    }

    if (error instanceof Error) {
      return new BitloopsCliError(error.message, error.message || fallbackMessage);
    }

    return new BitloopsCliError('Unknown Bitloops error.', fallbackMessage, String(error));
  }
}
