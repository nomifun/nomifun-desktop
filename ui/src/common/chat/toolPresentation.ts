import catalogue from '../../../../crates/backend/nomifun-agent-contracts/contracts/tool-presentation.json';
import { toDisplayText } from './displayText';

export interface ToolPresentationInput {
  name: string;
  capabilityId?: string;
  actionId?: string;
  input?: string;
  description?: string;
  origin?: { kind: 'mcp' | 'plugin'; name: string; toolName: string };
}

export interface ToolPresentation {
  title: string;
  target?: string;
  source?: string;
  receiptAction?: string;
  /** Exact diagnostics are kept apart from the translated title. */
  name: string;
  capabilityId?: string;
  actionId?: string;
}

const byName = new Map(catalogue.map((entry) => [entry.name, entry]));
const byAction = new Map(catalogue.filter((entry) => 'action_id' in entry)
  .map((entry) => [`${entry.capability_id}\0${entry.action_id}`, entry]));

const compact = (value: string): string => {
  const text = value.replace(/\s+/g, ' ').trim();
  const characters = Array.from(text);
  return characters.length > 120 ? `${characters.slice(0, 120).join('')}…` : text;
};

const humanize = (value: string): string => {
  const words = value.replace(/[_./-]+/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : '';
};

const readTarget = (input: string | undefined, fields: string[], isUrl: boolean): string | undefined => {
  if (!input || input.length > 256 * 1024) return undefined;
  try {
    const args: unknown = JSON.parse(input);
    for (const path of fields) {
      const value = path.split('.').reduce<unknown>((current, field) =>
        current && typeof current === 'object' ? (current as Record<string, unknown>)[field] : undefined, args);
      if (typeof value !== 'string' || !value.trim()) continue;
      if (isUrl) {
        try { return new URL(value).hostname || compact(value); } catch { return compact(value); }
      }
      return compact(value);
    }
  } catch {
    // Free-text descriptions remain available on tools without JSON input.
  }
  return undefined;
};

/** A view of typed facts; never a router, permission check, or retry identity. */
export const resolveToolPresentation = (tool: ToolPresentationInput, language = 'en-US'): ToolPresentation => {
  const chinese = language.toLowerCase().startsWith('zh');
  const entry = tool.origin ? undefined : tool.capabilityId && tool.actionId
    ? byAction.get(`${tool.capabilityId}\0${tool.actionId}`)
    : byName.get(tool.name);
  let title = entry?.titles[chinese ? 'zh-CN' : 'en-US'];
  let source: string | undefined;
  if (tool.origin) {
    source = `${tool.origin.kind === 'mcp' ? 'MCP' : chinese ? '插件' : 'Plugin'} · ${tool.origin.name}`;
    title = humanize(tool.origin.toolName) || (chinese ? '工具调用' : 'Tool call');
  } else if (!title) {
    const alias = tool.name.match(/^(mcp|plugin|platform)__(.+)$/);
    if (alias) {
      const segments = alias[2].split('__');
      if (/^(?:[a-f0-9]{20}|[a-z2-7]{16})$/.test(segments.at(-1) ?? '')) segments.pop();
      const [origin, ...action] = segments;
      source = `${alias[1] === 'mcp' ? 'MCP' : alias[1] === 'plugin' ? chinese ? '插件' : 'Plugin' : chinese ? '内置' : 'Built-in'} · ${origin}`;
      // Old platform aliases contain a truncated routing slug. The canonical
      // action above supplies their title; don't pretend to recover missing IDs.
      title = action.length ? humanize(action.join('_')) : chinese ? '工具调用' : 'Tool call';
    } else if (/^(mcp|plugin)_[a-f0-9]{40,64}$/.test(tool.name)) {
      source = tool.name.startsWith('mcp_') ? 'MCP' : chinese ? '插件' : 'Plugin';
      title = chinese ? '工具调用' : 'Tool call';
    } else {
      title = humanize(tool.name) || (chinese ? '工具调用' : 'Tool call');
    }
  }
  const rawDescription = toDisplayText(tool.description).trim();
  const description = rawDescription && ![tool.name, title].includes(rawDescription)
    ? compact(rawDescription) : undefined;
  const target = entry ? readTarget(tool.input, entry.target_fields, 'target_kind' in entry && entry.target_kind === 'url') : undefined;
  return {
    title, name: tool.name,
    ...(target || description ? { target: target || description } : {}),
    ...(source ? { source } : {}),
    ...(entry && 'receipt_action' in entry ? { receiptAction: entry.receipt_action } : {}),
    ...(tool.capabilityId ? { capabilityId: tool.capabilityId } : {}),
    ...(tool.actionId ? { actionId: tool.actionId } : {}),
  };
};

export const formatToolPresentationLabel = (presentation: ToolPresentation): string =>
  [presentation.title, presentation.target].filter(Boolean).join(' · ');

export const formatToolDiagnostics = (presentation: ToolPresentation): string =>
  [presentation.source, presentation.name, presentation.capabilityId, presentation.actionId].filter(Boolean).join('\n');
