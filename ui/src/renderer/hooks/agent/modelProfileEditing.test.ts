import { describe, expect, test } from 'bun:test';

import type { ModelProfile } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import {
  buildModelProfileUpsertRequest,
  editableModelTasks,
  editableModelTraits,
  isInferredModelProfile,
  primaryModelTask,
  visibleModelTaskBadges,
} from './modelProfileEditing';

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000001');

const profile = (source: ModelProfile['source'], tasks: ModelProfile['tasks'], traits: ModelProfile['traits'] = []): ModelProfile => ({
  provider_id: providerId,
  model: 'happyhorse-1.0',
  tasks,
  traits,
  params: {},
  source,
  updated_at: 1,
});

describe('model profile editing helpers', () => {
  test('shows inferred profiles pre-checked so the user can confirm them', () => {
    const inferred = profile('inferred', ['video_generation'], ['reasoning']);

    expect(editableModelTasks(inferred)).toEqual(['video_generation']);
    expect(editableModelTraits(inferred)).toEqual(['reasoning']);
    expect(visibleModelTaskBadges(inferred)).toEqual(['video_generation']);
    expect(isInferredModelProfile(inferred)).toBe(true);
    expect(isInferredModelProfile(profile('user', ['chat']))).toBe(false);
    expect(isInferredModelProfile(undefined)).toBe(false);
  });

  test('badges include every task, chat included', () => {
    const user = profile('user', ['chat', 'image_generation', 'video_generation'], ['vision_input']);

    expect(editableModelTasks(user)).toEqual(['chat', 'image_generation', 'video_generation']);
    expect(editableModelTraits(user)).toEqual(['vision_input']);
    expect(visibleModelTaskBadges(user)).toEqual(['chat', 'image_generation', 'video_generation']);
  });

  test('resolves the primary task for health probing', () => {
    expect(primaryModelTask(profile('inferred', ['speech_recognition', 'chat']))).toBe('speech_recognition');
    expect(primaryModelTask(profile('user', []))).toBeUndefined();
    expect(primaryModelTask(undefined)).toBeUndefined();
  });

  test('persists an empty user profile instead of falling back to a default task', () => {
    expect(buildModelProfileUpsertRequest(providerId, 'happyhorse-1.0', [], [])).toEqual({
      provider_id: providerId,
      model: 'happyhorse-1.0',
      tasks: [],
      traits: [],
      source: 'user',
    });
  });
});
