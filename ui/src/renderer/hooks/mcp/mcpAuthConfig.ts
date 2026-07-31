import type { IMcpServerTransport } from '@/common/config/storage';

const SENSITIVE_FIELD = /(api[_ -]?key|token|secret|password|authorization|bearer)/i;
const PLACEHOLDER_VALUE = /(\$\{[^}]+}|<[^>]+>|your[_ -]?[a-z0-9_ -]*(key|token|secret|password)|api[_ -]?key|token here|secret here|appbuilder api key)/i;
const WHOLE_URL_PLACEHOLDER = /^(\$\{[^}]+}|<[^>]+>|your[_ -]?[a-z0-9_ -]*(url|endpoint|server)|mcp[_ -]?url|server[_ -]?url)$/i;
const API_KEY_URL_BY_HOST: Record<string, string> = {
  'appbuilder.baidu.com': 'https://appbuilder.baidu.com/console',
};

const isPlaceholderValue = (value: string): boolean => {
  const trimmed = value.trim();
  if (!trimmed) return true;
  return PLACEHOLDER_VALUE.test(trimmed);
};

const addField = (fields: Set<string>, field: string) => {
  if (field.trim()) fields.add(field);
};

const scanStringRecord = (fields: Set<string>, prefix: string, record: Record<string, string> | undefined) => {
  if (!record) return;
  for (const [key, value] of Object.entries(record)) {
    if (SENSITIVE_FIELD.test(key) && isPlaceholderValue(value)) {
      addField(fields, `${prefix}.${key}`);
    }
    if (SENSITIVE_FIELD.test(value) && isPlaceholderValue(value)) {
      addField(fields, `${prefix}.${key}`);
    }
  }
};

const hasSensitiveRecordKey = (record: Record<string, string> | undefined): boolean =>
  record ? Object.keys(record).some((key) => SENSITIVE_FIELD.test(key)) : false;

const hasSensitiveUrlQuery = (url: string | undefined): boolean => {
  if (!url) return false;

  try {
    const parsed = new URL(url);
    for (const key of parsed.searchParams.keys()) {
      if (SENSITIVE_FIELD.test(key)) {
        return true;
      }
    }
  } catch {
    return false;
  }

  return false;
};

const scanUrl = (fields: Set<string>, url: string | undefined) => {
  if (!url) return;
  if (WHOLE_URL_PLACEHOLDER.test(url.trim())) {
    addField(fields, 'url');
  }

  try {
    const parsed = new URL(url);
    for (const [key, value] of parsed.searchParams.entries()) {
      if (SENSITIVE_FIELD.test(key) && isPlaceholderValue(value)) {
        addField(fields, `url.${key}`);
      }
    }
  } catch {
    // URL validity is handled by MCP import/edit validation.
  }
};

export const getMcpConfigurationFields = (transport: IMcpServerTransport): string[] => {
  const fields = new Set<string>();
  if (transport.type === 'stdio') {
    scanStringRecord(fields, 'env', transport.env);
    return [...fields];
  }

  scanUrl(fields, transport.url);
  scanStringRecord(fields, 'headers', transport.headers);
  return [...fields];
};

export const needsMcpApiConfiguration = (transport: IMcpServerTransport): boolean =>
  getMcpConfigurationFields(transport).length > 0;

export const hasMcpApiConfigurationInput = (transport: IMcpServerTransport): boolean => {
  if (transport.type === 'stdio') {
    return hasSensitiveRecordKey(transport.env);
  }

  return hasSensitiveUrlQuery(transport.url) || hasSensitiveRecordKey(transport.headers);
};

export const getMcpUrlTransportUrl = (transport: IMcpServerTransport): string | null => {
  if (transport.type === 'http' || transport.type === 'sse' || transport.type === 'streamable_http') {
    return transport.url;
  }
  return null;
};

export const getMcpApiKeyUrl = (transport: IMcpServerTransport): string | null => {
  const url = getMcpUrlTransportUrl(transport);
  if (!url) return null;

  try {
    const parsed = new URL(url);
    const host = parsed.hostname.toLowerCase();
    const knownUrl = API_KEY_URL_BY_HOST[host];
    if (knownUrl) return knownUrl;

    return hasSensitiveUrlQuery(url) ? parsed.origin : null;
  } catch {
    return null;
  }
};

export const supportsMcpOAuthLogin = (transport: IMcpServerTransport): boolean =>
  getMcpUrlTransportUrl(transport) !== null &&
  !needsMcpApiConfiguration(transport) &&
  !hasMcpApiConfigurationInput(transport);
