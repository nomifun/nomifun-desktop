import { expect, test } from 'bun:test';
import { scheduledTaskError } from './scheduledTaskError';

test('legacy summaries cannot classify an error or invent model request evidence', () => {
  const raw = JSON.stringify({ code: 'NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE', message: 'Old summary' });
  expect(scheduledTaskError(raw)).toEqual({ message: raw });
  expect(scheduledTaskError('resource binding failed')).toEqual({ message: 'resource binding failed' });
});

test('an explicit host configuration code can explain why a retained task needs reselection', () => {
  expect(scheduledTaskError('Task paused', 'CRON_CONFIGURATION_RESELECTION_REQUIRED')).toMatchObject({
    code: 'CRON_CONFIGURATION_RESELECTION_REQUIRED', retryable: false, ownership: 'nomifun',
  });
  expect(scheduledTaskError('Model removed', 'CRON_MODEL_RESELECTION_REQUIRED')).toMatchObject({
    code: 'CRON_MODEL_RESELECTION_REQUIRED', retryable: false, ownership: 'nomifun',
  });
  expect(scheduledTaskError('Unclassified summary', 'OTHER_FAILURE')).toEqual({ message: 'Unclassified summary' });
});
