import { describe, expect, it } from 'bun:test';
import {
  formatToolDisplayName,
  normalizeToolCall,
  normalizeToolGroup,
} from './normalizeToolCall';

describe('normalizeToolCall', () => {
  const nativeExit = (code = 1) => ({
    state: 'exited', exit_code: code, signal: null,
    output: { text: '0 pass, 1 fail', next_cursor: 14, retained_bytes: 14, dropped_bytes: 0 },
    cleanup: { interrupt_attempted: false, terminate_attempted: false, force_kill_attempted: false,
      reaped: true, elapsed_ms: 0, errors: [] },
    process_id: 'owned-process', success: code === 0,
  });

  it('keeps a real native nonzero exit visible as a command outcome', () => {
    const output = JSON.stringify(nativeExit());
    const result = normalizeToolCall({ type: 'tool_call', content: {
      call_id: 'diagnostic-test', name: 'exec_command', status: 'error', output,
    } } as any);
    expect(result?.status).toBe('error');
    expect(result?.commandExitCode).toBe(1);
    expect(result?.nonFatalFailure).toBe(true);
    expect(result?.output).toBe(output);
  });
  it('identifies a proven native launch refusal without erasing its error or diagnostic', () => {
    const receipt = {
      schema: 'nomifun.process-start-observation.v1', state: 'not_started',
      code: 'PROCESS_NOT_STARTED', user_code_started: false, success: false,
      message: 'The requested executable did not start.',
    };
    const normalize = (name: string, value: unknown, status = 'error') => normalizeToolCall({
      type: 'tool_call', content: { call_id: 'not-started', name, status, output: JSON.stringify(value) },
    } as any);
    const result = normalize('exec_command', receipt);
    expect(result?.commandNotStarted).toBe(true);
    expect(result?.status).toBe('error');
    expect(result?.notExecutedReason).toBeUndefined();
    expect(result?.nonFatalFailure).toBeUndefined();
    expect(result?.output).toBe(JSON.stringify(receipt));
    for (const value of [
      { ...receipt, user_code_started: true }, { ...receipt, success: true },
      { ...receipt, state: 'lost' }, { ...receipt, schema: 'remote.result' },
      { ...receipt, process_id: 'live' }, { ...receipt, exit_code: 1 },
      { ...receipt, signal: null }, { ...receipt, message: '' },
      { ...receipt, message: ' ' }, { ...receipt, message: 'x'.repeat(2049) },
    ]) {
      expect(normalize('exec_command', value)?.commandNotStarted).toBeUndefined();
    }
    expect(normalize('start_process', receipt)?.commandNotStarted).toBe(true);
    expect(normalize('poll_process', receipt)?.commandNotStarted).toBeUndefined();
    expect(normalize('remote_exec_command', receipt)?.commandNotStarted).toBeUndefined();
    expect(normalize('exec_command', receipt, 'completed')?.commandNotStarted).toBeUndefined();
  });

  it('shows a proven rejected process reference as unexecuted and preserves the diagnostic', () => {
    const receipt = { schema: 'nomifun.process-control-observation.v1', state: 'not_executed',
      code: 'PROCESS_REFERENCE_INVALID', operation: 'poll', control_applied: false, success: false,
      message: 'Process reference is not available in this exact turn; no control was applied.' };
    const normalize = (name: string, value: unknown) => normalizeToolCall({ type: 'tool_call', content: {
      call_id: 'wrong-reference', name, status: 'error', output: JSON.stringify(value),
    } } as any);
    const result = normalize('poll_process', receipt);
    expect(result?.notExecutedReason).toBe('process_reference');
    expect(result?.status).toBe('canceled');
    expect(result?.output).toBe(JSON.stringify(receipt));
    for (const value of [
      { ...receipt, control_applied: true }, { ...receipt, operation: 'cancel' },
      { ...receipt, state: 'lost' }, { ...receipt, process_id: 'foreign-id' },
    ]) expect(normalize('poll_process', value)?.notExecutedReason).toBeUndefined();
    expect(normalize('remote_poll_process', receipt)?.notExecutedReason).toBeUndefined();
  });

  it('does not relabel infrastructure, signal or cleanup failures as ordinary command exits', () => {
    for (const receipt of [
      { ...nativeExit(), state: 'timed_out' },
      { ...nativeExit(), signal: 9 },
      { ...nativeExit(), exit_code: -1 },
      { ...nativeExit(), cleanup: { ...nativeExit().cleanup, reaped: false } },
      { ...nativeExit(), cleanup: { ...nativeExit().cleanup, errors: ['cleanup unproven'] } },
      { ...nativeExit(), cleanup: { ...nativeExit().cleanup, force_kill_attempted: true } },
    ]) {
      const result = normalizeToolCall({ type: 'tool_call', content: {
        call_id: 'failed-native', name: 'exec_command', status: 'error', output: JSON.stringify(receipt),
      } } as any);
      expect(result?.commandExitCode).toBeUndefined();
      expect(result?.nonFatalFailure).toBeUndefined();
      expect(result?.status).toBe('error');
    }
    const remote = normalizeToolCall({ type: 'tool_call', content: {
      call_id: 'remote', name: 'remote_exec_command', status: 'error', output: JSON.stringify(nativeExit()),
    } } as any);
    expect(remote?.nonFatalFailure).toBeUndefined();
  });

  it('recognizes the native structured argument rejection without hiding its diagnostic', () => {
    const output = JSON.stringify({ status: 'not_executed', code: 'INVALID_TOOL_ARGUMENTS',
      tool: 'read_tool_history', issues: [{ schema_path: '/properties/id/pattern',
        expected: '^[0-9a-f]{64}$', parameter_path_template: '/id' }],
      message: 'No call in this batch was executed.' });
    const result = normalizeToolCall({ type: 'tool_call', content: {
      call_id: 'bad-history-id', name: 'read_tool_history', status: 'error', output,
    } } as any);
    expect(result?.notExecutedReason).toBe('invalid_arguments');
    expect(result?.output).toBe(output);
    const remote = normalizeToolCall({ type: 'tool_call', content: {
      call_id: 'remote', name: 'remote_read_tool_history', status: 'error', output,
    } } as any);
    expect(remote?.notExecutedReason).toBeUndefined();
  });

  it('preserves an explicit cancelled process receipt and its partial output', () => {
    const result = normalizeToolCall({type:'tool_call',content:{
      call_id:'cancel-call',name:'exec_command',status:'canceled',output:'STARTED',
    }} as any);
    expect(result?.status).toBe('canceled');
    expect(result?.output).toBe('STARTED');
  });

  it('preserves only structurally valid explicit retry identity', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-2',
        name: 'nomi_delegate',
        status: 'completed',
        args: { tasks: [] },
        retry: {
          retry_group_id: 'call-1',
          attempt_no: 2,
          retry_of_call_id: 'call-1',
        },
      },
    } as any);

    expect(result?.retry).toEqual({
      retryGroupId: 'call-1',
      attemptNo: 2,
      retryOfCallId: 'call-1',
    });
  });

  it('ignores tool_call messages without call_id', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: '',
        name: 'Glob',
        status: 'running',
        args: { pattern: '*.rs' },
      },
    } as any);

    expect(result).toBeUndefined();
  });

  it('marks ordinary non-zero Bash exits as non-fatal process outcomes', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-bash',
        name: 'Bash',
        status: 'error',
        args: { command: 'node test.js' },
        output: 'Exit code: 1\nSTDERR:\nTypeError: missing browser stub',
      },
    } as any);

    expect(result?.status).toBe('error');
    expect(result?.nonFatalFailure).toBe(true);
  });

  it('keeps an exact bounded search result inspectable without calling it an execution failure', () => {
    const output = JSON.stringify({
      kind: 'search_context_withheld',
      search_executed: true,
      snippets_withheld: true,
      notice: 'The search ran, but snippets need a narrower instruction scope.',
    });
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-search',
        name: 'search_files',
        status: 'error',
        args: { path: 'burst', query: 'needle', limit: 200 },
        output,
      },
    } as any);

    expect(result?.status).toBe('error');
    expect(result?.boundedResult).toBe('search_context_withheld');
    expect(result?.nonFatalFailure).toBe(true);
    expect(result?.output).toBe(output);
  });

  it('keeps malformed or mismatched bounded-search claims fatal', () => {
    for (const [name, value] of [
      ['search_files', { kind: 'search_context_withheld', search_executed: false, snippets_withheld: true, notice: 'x' }],
      ['read_file', { kind: 'search_context_withheld', search_executed: true, snippets_withheld: true, notice: 'x' }],
      ['search_files', { kind: 'search_context_withheld', search_executed: true, snippets_withheld: true, notice: 'x', extra: true }],
    ] as const) {
      const result = normalizeToolCall({
        type: 'tool_call',
        content: { call_id: `call-${name}`, name, status: 'error', output: JSON.stringify(value) },
      } as any);
      expect(result?.boundedResult).toBeUndefined();
      expect(result?.nonFatalFailure).toBeUndefined();
    }
  });

  it('marks prior-error barrier results as skipped cancellations', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-skipped',
        name: 'Bash',
        status: 'error',
        args: { command: 'find /workspace -maxdepth 2 -type d' },
        output:
          'Skipped because a previous tool call in this assistant turn failed. Inspect the failed result first.',
      },
    } as any);

    expect(result?.status).toBe('canceled');
    expect(result?.skipped).toBe(true);
    expect(result?.nonFatalFailure).toBeUndefined();
  });

  it('classifies standardized local invalid-argument rejections as not executed', () => {
    const name = 'mcp__nomifun-desktop__nomi_delegate__anxmvqfkcuzfi4mq';
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-invalid-arguments',
        name,
        status: 'error',
        args: null,
        output:
          `Invalid arguments for tool '${name}': JSON Schema validation failed: at $: ` +
          '{"max_parallel":"4"} is not valid. Correct the arguments and retry; the tool was not executed.',
      },
    } as any);

    expect(result?.status).toBe('canceled');
    expect(result?.notExecutedReason).toBe('invalid_arguments');
    expect(result?.skipped).toBeUndefined();
    expect(result?.nonFatalFailure).toBeUndefined();
    expect(result?.input).toBeUndefined();
  });

  it('shows a local runtime preflight deferral as unexecuted without hiding its diagnostic', () => {
    const output = 'Operations not executed: instruction scope changed before write';
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-preflight', name: 'write_file', status: 'error',
        args: { path: 'snake_game.html' }, output,
      },
    } as any);
    expect(result?.status).toBe('canceled');
    expect(result?.notExecutedReason).toBe('runtime_preflight');
    expect(result?.output).toBe(output);
  });

  it('treats the local command-shape rejection as not executed', () => {
    const output = 'Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE): Process launch failed. The command field must contain only the executable; put options in args. No successful launch was reported.';
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-command-shape', name: 'exec_command', status: 'error',
        args: { command: 'ls -la' }, output,
      },
    } as any);

    expect(result?.status).toBe('canceled');
    expect(result?.notExecutedReason).toBe('runtime_preflight');
    expect(result?.output).toBe(output);
  });

  it('keeps remote and actual local failures red even if the text resembles a preflight', () => {
    for (const [name, output] of [
      ['mcp__server__write_file__abcdefghijklmnop', 'Operations not executed: remote service error'],
      ['write_file', 'Permission denied while writing snake_game.html'],
    ]) {
      const result = normalizeToolCall({
        type: 'tool_call',
        content: { call_id: `call-${name}`, name, status: 'error', output },
      } as any);
      expect(result?.status).toBe('error');
      expect(result?.notExecutedReason).toBeUndefined();
    }
  });

  it('keeps remote failures fatal even when their arguments are null', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-remote-error',
        name: 'mcp__server__remote_action__abcdefghijklmnop',
        status: 'error',
        args: null,
        output: 'Remote service rejected the request: invalid arguments',
      },
    } as any);

    expect(result?.status).toBe('error');
    expect(result?.notExecutedReason).toBeUndefined();
  });

  it('does not accept a standardized rejection that names a different tool', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-mismatched-tool',
        name: 'mcp__server__actual__abcdefghijklmnop',
        status: 'error',
        args: null,
        output:
          "Invalid arguments for tool 'mcp__server__other__bcdefghijklmnopq': expected a JSON object. " +
          'Correct the arguments and retry; the tool was not executed.',
      },
    } as any);

    expect(result?.status).toBe('error');
    expect(result?.notExecutedReason).toBeUndefined();
  });

  it('formats canonical MCP aliases as stable server/tool labels', () => {
    expect(formatToolDisplayName('mcp__nomifun-desktop__nomi_delegate__anxmvqfkcuzfi4mq')).toBe(
      'nomifun-desktop/nomi_delegate'
    );
    expect(formatToolDisplayName('mcp__server__read_file')).toBe('server/read_file');
    expect(formatToolDisplayName('Bash')).toBe('Bash');
  });

  const infrastructureFailures = [
    'Command timed out after 120000ms.\nPartial output:\nRESULT_PASS',
    'Command was cancelled.\nSTDOUT:\npartial',
    'Failed to execute command: executable not found (spawn_failed)',
    'Command cleanup is unproven (pid=42, state=Lost). Do not blindly retry.',
    'The turn ended before this tool completed: channel_closed',
    'Exit code: -1\nSignal: 9\nOUTPUT:',
    'Exit code: 1\nOUTPUT:\nCleanup diagnostics: exact cleanup was not proven',
  ];

  for (const [index, output] of infrastructureFailures.entries()) {
    it(`keeps Bash infrastructure failure ${index + 1} fatal`, () => {
      const result = normalizeToolCall({
        type: 'tool_call',
        content: {
          call_id: 'call-bash',
          name: 'Bash',
          status: 'error',
          args: { command: 'node test.js' },
          output,
        },
      } as any);

      expect(result?.nonFatalFailure).toBeUndefined();
    });
  }

  it('marks only explicit direct read/search probe misses as non-fatal', () => {
    for (const [name, output] of [
      ['Read', 'Failed to read file missing.file: No such file or directory (os error 2)'],
      ['Glob', 'No files matched the pattern'],
      ['Grep', 'No matches found'],
    ]) {
      const result = normalizeToolCall({
        type: 'tool_call',
        content: {
          call_id: `call-${name}`,
          name,
          status: 'error',
          args: { path: 'missing.file' },
          output,
        },
      } as any);

      expect(result?.nonFatalFailure).toBe(true);
    }
  });

  it('keeps direct probe permission and syntax failures fatal', () => {
    for (const [name, output] of [
      ['Read', 'Failed to read file secret.txt: Permission denied (os error 13)'],
      ['Glob', 'Invalid glob pattern: Pattern syntax error near position 2'],
      ['Grep', 'rg error: permission denied'],
    ]) {
      const result = normalizeToolCall({
        type: 'tool_call',
        content: {
          call_id: `call-${name}`,
          name,
          status: 'error',
          args: { path: 'secret.txt' },
          output,
        },
      } as any);

      expect(result?.nonFatalFailure).toBeUndefined();
    }
  });

  it('keeps interrupted direct probes fatal', () => {
    const result = normalizeToolCall({
      type: 'tool_call',
      content: {
        call_id: 'call-read',
        name: 'Read',
        status: 'error',
        args: { path: 'config.json' },
        output: 'The turn ended before this tool completed: channel_closed',
      },
    } as any);

    expect(result?.nonFatalFailure).toBeUndefined();
  });
});

describe('normalizeToolGroup', () => {
  it('keeps failed tool groups fatal', () => {
    const [result] = normalizeToolGroup({
      type: 'tool_group',
      content: [
        {
          call_id: 'call-edit',
          name: 'Edit',
          status: 'Error',
          description: 'app.ts',
        },
      ],
    } as any);

    expect(result.status).toBe('error');
    expect(result.nonFatalFailure).toBeUndefined();
  });
});
