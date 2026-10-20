import type { ProductAgentSelection } from '@/common/types/agentPlatform';
import { parseAgentPresetId } from '@/common/types/ids';
import type { AgentSelection } from './types';

export const agentSelectionToWire = (selection: AgentSelection): ProductAgentSelection => selection.kind === 'template'
  ? { kind: 'template', template_key: selection.templateKey }
  : { kind: 'preset', preset_id: selection.presetId };

export const agentSelectionFromWire = (selection: ProductAgentSelection): AgentSelection => selection.kind === 'template'
  ? { kind: 'template', templateKey: selection.template_key }
  : { kind: 'preset', presetId: parseAgentPresetId(selection.preset_id) };

export const sameAgentSelection = (left: ProductAgentSelection | undefined, right: ProductAgentSelection | undefined): boolean => {
  if (!left || !right) return left === right;
  return left.kind === 'template' && right.kind === 'template'
    ? left.template_key === right.template_key
    : left.kind === 'preset' && right.kind === 'preset' && left.preset_id === right.preset_id;
};
