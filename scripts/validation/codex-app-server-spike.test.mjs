import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import {
  assertSelfTest,
  extractTopLevelMethodValues,
  inspectUpstream,
  parseArgs,
  redactValue,
  runLiveProbe,
} from './codex-app-server-spike.mjs';

describe('SL-S2-10 Codex app-server spike harness', () => {
  test('self-test only reports harness coverage, not upstream evidence', () => {
    expect(assertSelfTest()).toEqual(
      expect.objectContaining({ status: 'self-test-pass' }),
    );
  });

  test('extracts only top-level protocol method values', () => {
    expect(
      extractTopLevelMethodValues({
        oneOf: [
          { properties: { method: { enum: ['initialize'] } } },
          { properties: { method: { enum: ['thread/start'] } } },
        ],
        definitions: {
          Nested: { properties: { method: { enum: ['not-a-wire-method'] } } },
        },
      }),
    ).toEqual(['initialize', 'thread/start']);
  });

  test('requires explicit acknowledgement before model turn smoke', () => {
    expect(() => parseArgs(['--run-turn-cancel'])).toThrow(
      '--run-turn-cancel requires --allow-live-model',
    );
  });

  test('redacts credentials and filesystem paths while retaining protocol fields', () => {
    expect(
      redactValue({
        apiKey: 'do-not-print',
        cwd: 'C:\\secret\\workspace',
        method: 'turn/interrupt',
      }),
    ).toEqual({
      apiKey: '<redacted>',
      cwd: '<path>',
      method: 'turn/interrupt',
    });
  });

  test('missing upstream stays blocked and never becomes a PASS', () => {
    const report = inspectUpstream({
      upstreamDir: join(tmpdir(), `nomifun-missing-upstream-${Date.now()}`),
      pinnedCommit: 'dc2ccc6843abb09c9d297862dc10b6bd12a3935d',
    });
    expect(report.status).toBe('blocked');
    expect(report.status).not.toBe('pass');
    expect(report.blockers.length).toBeGreaterThan(0);
  });

  test('fake transport exercises live framing, thread, interrupt, completion, and close', async () => {
    const fakeServer = `
      const readline = require('node:readline');
      const send = (value) =>
        process.stdout.write(JSON.stringify(value) + String.fromCharCode(10));
      const input = readline.createInterface({ input: process.stdin });
      input.on('line', (line) => {
        if (!line.trim()) return;
        const request = JSON.parse(line);
        if (request.method === 'initialize') {
          send({ id: request.id, result: { userAgent: 'fake', platformOs: 'test' } });
        } else if (request.method === 'thread/start') {
          send({ id: request.id, result: { thread: { id: 'thread-1' } } });
          send({ method: 'thread/started', params: { thread: { id: 'thread-1' } } });
        } else if (request.method === 'turn/start') {
          send({ id: request.id, result: { turn: { id: 'turn-1' } } });
          send({ method: 'turn/started', params: { threadId: 'thread-1', turn: { id: 'turn-1' } } });
        } else if (request.method === 'turn/interrupt') {
          send({ id: request.id, result: {} });
          send({ method: 'turn/completed', params: { turn: { id: 'turn-1', status: 'interrupted' } } });
        }
      });
    `;
    const report = await runLiveProbe(
      {
        binary: process.execPath,
        runThreadStart: true,
        runTurnCancel: true,
        allowLiveModel: true,
        codexHome: tmpdir(),
        timeoutMs: 2_000,
      },
      {
        command: process.execPath,
        args: ['-e', fakeServer],
      },
    );
    expect(report.checks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: 'live:initialize', status: 'observed' }),
        expect.objectContaining({ id: 'live:thread-start', status: 'observed' }),
        expect.objectContaining({ id: 'live:turn-interrupt', status: 'observed' }),
        expect.objectContaining({ id: 'live:turn-completed', status: 'observed' }),
        expect.objectContaining({ id: 'live:process-close', status: 'observed' }),
      ]),
    );
  });
});
