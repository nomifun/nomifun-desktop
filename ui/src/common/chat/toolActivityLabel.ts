import { formatToolPresentationLabel, resolveToolPresentation } from '@/common/chat/toolPresentation';

/** The same admitted tool identity and action labels used by the transcript. */
export function toolActivityLabel(data: unknown, language: string): string | null {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
  const tool = data as { name?: unknown; capability_id?: unknown; action_id?: unknown; description?: unknown };
  if (typeof tool.name !== 'string' || !tool.name) return null;
  return formatToolPresentationLabel(resolveToolPresentation({ name: tool.name,
    ...(typeof tool.capability_id === 'string' ? { capabilityId: tool.capability_id } : {}),
    ...(typeof tool.action_id === 'string' ? { actionId: tool.action_id } : {}),
    ...(typeof tool.description === 'string' ? { description: tool.description } : {}),
  }, language));
}
