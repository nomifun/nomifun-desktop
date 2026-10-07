import type { TChatConversation } from '@/common/config/storage';
import type { AgentResolvedSnapshot, DigestHex } from '@/common/types/agentPlatform';
import { parseAgentPresetId, parseConversationId, parseEntityId, parseProviderId, parseSshHostId, type SshHostId } from '@/common/types/ids';

export const firstSshHostId = parseSshHostId('0190f5fe-7c00-7a00-8000-000000000071');
export const secondSshHostId = parseSshHostId('0190f5fe-7c00-7a00-8000-000000000072');
export const missingSshHostId = parseSshHostId('0190f5fe-7c00-7a00-8000-000000000073');

export const makeSshConversation = (
  hostId: SshHostId | null = firstSshHostId,
  bindingVersion = 1,
): TChatConversation => {
  const presetId = parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000074');
  const snapshot: AgentResolvedSnapshot = {
    canonical_binding: {
      preset_revision_ref: { preset_id: presetId, revision: 1, revision_digest: 'a'.repeat(64) as DigestHex },
      resolved_snapshot_ref: {
        snapshot_id: parseEntityId('resolved-snapshot', '0190f5fe-7c00-7a00-8000-000000000075'),
        snapshot_digest: 'b'.repeat(64) as DigestHex,
      },
      typed_resource_bindings: hostId ? [{
        binding_id: `ssh-host:${hostId}`, resource_kind: 'ssh_host', resource_id: hostId,
        owner_id: 'fixture-owner', operations: ['ssh/exec'],
      }] : [],
      binding_version: bindingVersion,
    },
    preset_id: presetId, preset_revision: 1, preset_name: 'SSH Agent', instructions: '',
    included_skills: [], excluded_auto_skills: [], enabled_capabilities: ['ssh'],
    enabled_capability_actions: { ssh: ['ssh/exec'] }, required_resource_kinds: ['ssh_host'],
    knowledge_policy: { enabled: false, writeback: false, grounded: false }, warnings: [],
  };
  return {
    id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000076'),
    type: 'nomi', name: 'Remote work', created_at: 1000, modified_at: 2000,
    extra: { workspace: '' }, preset_id: presetId, agent_snapshot: snapshot,
    model: {
      id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000077'), platform: 'openai',
      name: 'Test provider', base_url: 'https://example.test/v1', auth_scheme: 'bearer',
      has_credentials: false, use_model: 'fixture-model',
    },
  };
};
