import { strict as assert } from 'node:assert';
import { suite, test } from 'mocha';

import {
  BitloopsCliClient,
  BitloopsCliError,
  CommandExecutionError,
  CommandExecutor,
} from '../../bitloopsCli';

class FakeExecutor implements CommandExecutor {
  constructor(
    private readonly handler: (
      command: string,
      args: string[],
      cwd: string,
    ) => Promise<{ stdout: string; stderr: string }>,
  ) {}

  execute(
    command: string,
    args: string[],
    options: { cwd: string },
  ): Promise<{ stdout: string; stderr: string }> {
    return this.handler(command, args, options.cwd);
  }
}

suite('bitloopsCli', () => {
  test('healthy daemon and query execution return parsed JSON', async () => {
    let daemonChecks = 0;
    const client = new BitloopsCliClient({
      executor: new FakeExecutor(async (_command, args) => {
        if (args[0] === 'daemon') {
          daemonChecks += 1;
          return {
            stdout: JSON.stringify({ runtime: { url: 'https://127.0.0.1:5667' } }),
            stderr: '',
          };
        }

        return {
          stdout: JSON.stringify({ selectArtefacts: { count: 0, artefacts: [] } }),
          stderr: '',
        };
      }),
      getCliPath: () => 'bitloops',
      now: () => 1,
    });

    const result = await client.executeGraphqlQuery<{ selectArtefacts: { count: number } }>(
      '/repo',
      '{ selectArtefacts(by: { search: "x" }) { count } }',
    );

    assert.equal(result.selectArtefacts.count, 0);
    assert.equal(daemonChecks, 1);
  });

  test('missing CLI reports a helpful user message', async () => {
    const client = new BitloopsCliClient({
      executor: new FakeExecutor(async () => {
        throw new CommandExecutionError('missing binary', '', '', undefined, 'ENOENT');
      }),
      getCliPath: () => '/custom/bitloops',
    });

    await assert.rejects(
      () => client.ensureDaemonAvailable('/repo'),
      (error: unknown) =>
        error instanceof BitloopsCliError &&
        error.userMessage.includes('/custom/bitloops') &&
        error.userMessage.includes('bitloops.cliPath'),
    );
  });

  test('daemon unavailable reports the status command guidance', async () => {
    const client = new BitloopsCliClient({
      executor: new FakeExecutor(async () => ({
        stdout: JSON.stringify({ runtime: null }),
        stderr: '',
      })),
    });

    await assert.rejects(
      () => client.ensureDaemonAvailable('/repo'),
      (error: unknown) =>
        error instanceof BitloopsCliError &&
        error.userMessage.includes('Run `bitloops status`'),
    );
  });
});
