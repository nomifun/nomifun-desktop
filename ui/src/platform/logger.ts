/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * In-house logger facade.
 *
 * Replaces the logger surface of the former third-party platform package.
 * Only `config` and `provider` are used by the app; the rest of the original
 * surface (log/info/warn/error/performance/logPath) is unused and omitted.
 */

interface LogEntry {
  type: 'log' | 'info' | 'warn' | 'error';
  logs: any[];
}

interface LogProvider {
  log(entry: LogEntry): void;
  path(): Promise<string>;
}

let printToConsole = false;
let sink: LogProvider | null = null;

const emit = (type: LogEntry['type'], logs: any[]): void => {
  if (sink) sink.log({ type, logs });
  if (printToConsole) console[type](...logs);
};

export const config = (extra?: { print?: boolean }): { logPath: Promise<string> } => {
  printToConsole = !!extra?.print;
  return { logPath: sink ? sink.path() : Promise.resolve('') };
};

export const provider = (config: LogProvider): (() => void) => {
  sink = config;
  return () => {
    if (sink === config) sink = null;
  };
};

export const log = (...args: any[]): void => emit('log', args);
export const info = (...args: any[]): void => emit('info', args);
export const warn = (...args: any[]): void => emit('warn', args);
export const error = (...args: any[]): void => emit('error', args);
