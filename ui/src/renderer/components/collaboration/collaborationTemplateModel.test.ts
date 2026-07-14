import { describe, expect, test } from 'bun:test';
import type { TAgentExecutionTemplateDetail } from '@/common/types/agentExecution/agentExecutionTemplateTypes';
import {
  templateContainsModel,
  templateParticipantModels,
  toAppliedCollaborationTemplate,
} from './collaborationTemplateModel';

const detail = {
  id: 'template-1',
  name: 'Review plan',
  participants: [
    { provider_id: 'provider-a', model: 'model-a' },
    { provider_id: 'provider-a', model: 'model-a' },
    { provider_id: 'provider-b', model: 'model-b' },
    { provider_id: null, model: null },
  ],
} as TAgentExecutionTemplateDetail;

describe('collaboration template model', () => {
  test('derives one canonical model set from executable participants', () => {
    expect(templateParticipantModels(detail)).toEqual([
      { provider_id: 'provider-a', model: 'model-a' },
      { provider_id: 'provider-b', model: 'model-b' },
    ]);
    expect(
      templateContainsModel(detail, { provider_id: 'provider-b', model: 'model-b' }),
    ).toBe(true);
    expect(
      templateContainsModel(detail, { provider_id: 'provider-c', model: 'model-c' }),
    ).toBe(false);
  });

  test('keeps participant count distinct from the deduplicated model authority', () => {
    expect(toAppliedCollaborationTemplate(detail)).toEqual({
      id: 'template-1',
      name: 'Review plan',
      participantCount: 4,
      models: [
        { provider_id: 'provider-a', model: 'model-a' },
        { provider_id: 'provider-b', model: 'model-b' },
      ],
    });
  });
});
