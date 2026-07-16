declare const extensionMcpContributionKeyBrand: unique symbol;

export type ExtensionMcpContributionKey = string & {
  readonly [extensionMcpContributionKeyBrand]: 'extension-mcp-contribution';
};

/**
 * Read-only MCP contribution exposed by an extension.
 *
 * `contributionKey` is intentionally opaque: extension contributions are not
 * repository-backed MCP entities and must never be passed to APIs that require
 * a canonical `McpServerId`.
 */
export interface ExtensionMcpServerContribution {
  readonly contributionKey: ExtensionMcpContributionKey;
  readonly name: string;
  readonly description?: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === 'object' && !Array.isArray(value);

const nonEmptyString = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim() ? value : undefined;

const extensionContributionKey = (value: unknown): ExtensionMcpContributionKey | undefined => {
  const key = nonEmptyString(value);
  if (!key || key.trim() !== key || !key.startsWith('ext-') || !key.slice('ext-'.length).trim()) return undefined;
  return key as ExtensionMcpContributionKey;
};

export const extensionMcpUiKey = (key: ExtensionMcpContributionKey): `extension:${string}` => `extension:${key}`;

const parseExtensionMcpServer = (value: unknown): ExtensionMcpServerContribution | undefined => {
  if (!isRecord(value)) return undefined;

  const contributionKey = extensionContributionKey(value.id);
  const name = nonEmptyString(value.name);
  const description = nonEmptyString(value.description);
  if (!contributionKey || !name) return undefined;

  return {
    contributionKey,
    name,
    ...(description ? { description } : {}),
  };
};

/** Parse each contribution independently so one malformed extension cannot hide its siblings. */
export const parseExtensionMcpServers = (values: readonly unknown[]): ExtensionMcpServerContribution[] => {
  const seen = new Set<string>();
  const contributions: ExtensionMcpServerContribution[] = [];

  for (const value of values) {
    const parsed = parseExtensionMcpServer(value);
    if (!parsed || seen.has(parsed.contributionKey)) continue;

    seen.add(parsed.contributionKey);
    contributions.push(parsed);
  }

  return contributions;
};
