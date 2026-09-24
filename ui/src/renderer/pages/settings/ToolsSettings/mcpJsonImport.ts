import type { IMcpServerTransport } from '@/common/config/storage';

type McpJsonImportErrorKey =
  | 'settings.mcpJsonFormatError'
  | 'settings.mcpJsonBareServerError'
  | 'settings.mcpJsonUrlRequiredError'
  | 'settings.mcpJsonStdioCommandRequiredError';

export type ParsedMcpJsonServer = {
  name: string;
  description: string;
  transport: IMcpServerTransport;
  originalConfig: Record<string, unknown>;
};

export type McpJsonImportResult =
  | {
      isValid: true;
      servers: ParsedMcpJsonServer[];
    }
  | {
      isValid: false;
      errorKey: McpJsonImportErrorKey;
    };

const IMPORTED_DESCRIPTION = 'Imported from JSON';

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const hasOwn = (value: Record<string, unknown>, key: string) => Object.prototype.hasOwnProperty.call(value, key);

const stringOrUndefined = (value: unknown): string | undefined => (typeof value === 'string' ? value : undefined);

const nonEmptyString = (value: unknown): string | undefined => {
  const text = stringOrUndefined(value);
  return text?.trim() ? text : undefined;
};

const toStringRecord = (value: unknown): Record<string, string> | undefined => {
  if (!isRecord(value)) return undefined;

  const entries = Object.entries(value);
  if (!entries.every(([, item]) => typeof item === 'string')) return undefined;
  return Object.fromEntries(entries) as Record<string, string>;
};

const configValue = (
  transportConfig: Record<string, unknown>,
  config: Record<string, unknown>,
  keys: string[]
): unknown => {
  for (const key of keys) {
    if (hasOwn(transportConfig, key)) return transportConfig[key];
  }
  for (const key of keys) {
    if (hasOwn(config, key)) return config[key];
  }
  return undefined;
};

const normalizeTransportType = (value: unknown): IMcpServerTransport['type'] | undefined | null => {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || !value.trim()) return null;

  switch (value.trim().toLowerCase().replace(/[\s_-]/g, '')) {
    case 'stdio':
      return 'stdio';
    case 'sse':
      return 'sse';
    case 'http':
      return 'http';
    case 'streamablehttp':
      return 'streamable_http';
    default:
      return null;
  }
};

const normalizeArgs = (value: unknown): string[] | undefined => {
  if (value === undefined) return [];
  if (typeof value === 'string') return [value];
  if (Array.isArray(value) && value.every((item) => typeof item === 'string')) return value;
  return undefined;
};

const looksLikeBareServer = (value: Record<string, unknown>): boolean =>
  [
    'command',
    'args',
    'env',
    'url',
    'baseUrl',
    'base_url',
    'endpoint',
    'serverUrl',
    'server_url',
    'headers',
    'type',
    'transport',
    'description',
  ].some((key) => hasOwn(value, key));

const normalizeArrayServer = (
  serverItem: unknown
):
  | { isValid: true; name: string; config: Record<string, unknown> }
  | { isValid: false; errorKey: McpJsonImportErrorKey } => {
  if (!isRecord(serverItem)) {
    return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  const name = nonEmptyString(serverItem.name);
  if (!name) {
    return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  const { name: _name, ...config } = serverItem;
  return { isValid: true, name, config };
};

const parseTransport = (
  config: Record<string, unknown>
): { isValid: true; transport: IMcpServerTransport } | { isValid: false; errorKey: McpJsonImportErrorKey } => {
  const transportObject = isRecord(config.transport) ? config.transport : undefined;
  const transportConfig = transportObject ?? config;
  const typeFromTransport = transportObject?.type ?? config.transport;
  const transportType = normalizeTransportType(config.type ?? typeFromTransport);

  if (transportType === null) {
    return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  if (hasOwn(transportConfig, 'command') || transportType === 'stdio') {
    const command = nonEmptyString(configValue(transportConfig, config, ['command']));
    if (!command) {
      return { isValid: false, errorKey: 'settings.mcpJsonStdioCommandRequiredError' };
    }

    const args = normalizeArgs(configValue(transportConfig, config, ['args']));
    if (!args) {
      return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
    }

    const rawEnv = configValue(transportConfig, config, ['env']);
    const env = rawEnv === undefined ? {} : toStringRecord(rawEnv);
    if (!env) {
      return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
    }

    return {
      isValid: true,
      transport: {
        type: 'stdio',
        command,
        args,
        env,
      },
    };
  }

  const url = nonEmptyString(
    configValue(transportConfig, config, ['url', 'baseUrl', 'base_url', 'endpoint', 'serverUrl', 'server_url'])
  );
  if (!url) {
    return { isValid: false, errorKey: 'settings.mcpJsonUrlRequiredError' };
  }

  const rawHeaders = configValue(transportConfig, config, ['headers']);
  const headers = rawHeaders === undefined ? undefined : toStringRecord(rawHeaders);
  if (rawHeaders !== undefined && !headers) {
    return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  const normalizedType = transportType ?? (url.toLowerCase().includes('/sse') ? 'sse' : 'http');
  return {
    isValid: true,
    transport: {
      type: normalizedType,
      url,
      headers,
    },
  };
};

const parseServer = (
  name: string,
  config: Record<string, unknown>
): { isValid: true; server: ParsedMcpJsonServer } | { isValid: false; errorKey: McpJsonImportErrorKey } => {
  const transportResult = parseTransport(config);
  if (transportResult.isValid === false) return transportResult;

  return {
    isValid: true,
    server: {
      name,
      description: stringOrUndefined(config.description) || IMPORTED_DESCRIPTION,
      transport: transportResult.transport,
      originalConfig: config,
    },
  };
};

export const parseMcpJsonImport = (config: unknown): McpJsonImportResult => {
  const rawServers = isRecord(config) && hasOwn(config, 'mcpServers') ? config.mcpServers : config;

  if (Array.isArray(rawServers)) {
    const servers: ParsedMcpJsonServer[] = [];
    for (const rawServer of rawServers) {
      const normalized = normalizeArrayServer(rawServer);
      if (normalized.isValid === false) return normalized;

      const parsed = parseServer(normalized.name, normalized.config);
      if (parsed.isValid === false) return parsed;

      servers.push(parsed.server);
    }

    return servers.length > 0
      ? { isValid: true, servers }
      : { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  if (!isRecord(rawServers)) {
    return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
  }

  if (rawServers === config && looksLikeBareServer(rawServers)) {
    return { isValid: false, errorKey: 'settings.mcpJsonBareServerError' };
  }

  const servers: ParsedMcpJsonServer[] = [];
  for (const [name, serverConfig] of Object.entries(rawServers)) {
    if (!name.trim() || !isRecord(serverConfig)) {
      return { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
    }

    const parsed = parseServer(name, serverConfig);
    if (parsed.isValid === false) return parsed;

    servers.push(parsed.server);
  }

  return servers.length > 0 ? { isValid: true, servers } : { isValid: false, errorKey: 'settings.mcpJsonFormatError' };
};
