import catalogue from '../../../../crates/backend/nomifun-agent-contracts/contracts/tool-presentation.json';
import { toDisplayText } from './displayText';

export interface ToolPresentationInput {
  name: string;
  capabilityId?: string;
  actionId?: string;
  description?: string;
  origin?: { kind: 'mcp' | 'plugin'; name: string; toolName: string };
}

export interface ToolPresentation {
  title: string;
  target?: string;
  source?: string;
  receiptAction?: string;
  /** Which read-only detail the conversation can show for this invocation. */
  detailPolicy: 'payload' | 'files' | 'receipt';
  /** Exact diagnostics are kept apart from the translated title. */
  name: string;
  capabilityId?: string;
  actionId?: string;
}

/** The Runtime reserves this name; explicit host/external identities remain tools. */
export const isNativeCompletionReport = (tool: Pick<ToolPresentationInput, 'name' | 'capabilityId' | 'actionId' | 'origin'>): boolean =>
  tool.name === 'report_completion' && tool.capabilityId === undefined && tool.actionId === undefined && !tool.origin;

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

/** A view of typed facts; never a router, permission check, or retry identity. */
export const resolveToolPresentation = (tool: ToolPresentationInput, language = 'en-US'): ToolPresentation => {
  const chinese = language.toLowerCase().startsWith('zh');
  const explicitIdentity = tool.capabilityId !== undefined || tool.actionId !== undefined;
  const entry = tool.origin ? undefined : explicitIdentity
    ? tool.capabilityId && tool.actionId ? byAction.get(`${tool.capabilityId}\0${tool.actionId}`) : undefined
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
  const receiptAction = entry && 'receipt_action' in entry ? entry.receipt_action : undefined;
  const detailPolicy = isNativeCompletionReport(tool) ? 'receipt'
    : receiptAction === 'read_files' || receiptAction === 'edit_files'
      || tool.capabilityId === 'ssh' && (tool.actionId === 'ssh/fs.read' || tool.actionId === 'ssh/fs.write') ? 'files' : 'payload';
  return {
    title, name: tool.name, detailPolicy,
    ...(description ? { target: description } : {}),
    ...(source ? { source } : {}),
    ...(receiptAction ? { receiptAction } : {}),
    ...(tool.capabilityId ? { capabilityId: tool.capabilityId } : {}),
    ...(tool.actionId ? { actionId: tool.actionId } : {}),
  };
};

export const formatToolPresentationLabel = (presentation: ToolPresentation): string =>
  [presentation.title, presentation.target].filter(Boolean).join(' · ');
