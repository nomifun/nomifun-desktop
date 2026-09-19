import type { IMcpServer } from '@/common/config/storage';
import type { SkillInfo } from '@/common/types/skill';

export type SessionSkillOption = Omit<SkillInfo, 'source'> & {
  source: SkillInfo['source'] | 'extension';
  auto: boolean;
};

export type SessionCapabilityCatalog = {
  skills: SessionSkillOption[];
  autoSkillNames: ReadonlySet<string>;
  mcpServers: IMcpServer[];
};

export type SessionCapabilityDraft = {
  skillNames: string[];
  mcpServerIds: string[];
};
