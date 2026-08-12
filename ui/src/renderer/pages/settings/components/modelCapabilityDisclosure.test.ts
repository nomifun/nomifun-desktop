/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { CapabilityValidationIssue } from './modelCapabilityDisclosure';
import {
  compactCapabilityUrlSummary,
  createCapabilityDisclosureState,
  getSettledCapabilityValidationErrors,
  syncCapabilityDisclosureState,
  toggleCapabilityDisclosure,
} from './modelCapabilityDisclosure';

const tasks = (...values: ModelTask[]): ModelTask[] => values;

describe('model capability disclosure state', () => {
  test('shows only a safe URL origin in collapsed summaries', () => {
    expect(
      compactCapabilityUrlSummary(
        'https://user:password@gateway.example.com/private/api-token/v1?key=secret#fragment'
      )
    ).toBe('https://gateway.example.com');
    expect(compactCapabilityUrlSummary('wss://realtime.example.com/session/token')).toBe(
      'wss://realtime.example.com'
    );
    expect(compactCapabilityUrlSummary('/relative/private/token')).toBe('');
  });

  test('keeps valid capabilities collapsed by default', () => {
    const state = createCapabilityDisclosureState(tasks('chat', 'video_generation'));

    expect([...state.expandedTasks]).toEqual([]);
    expect([...state.errorTasks]).toEqual([]);
  });

  test('toggles one capability without mutating the previous state', () => {
    const initial = createCapabilityDisclosureState(tasks('chat', 'video_generation'));
    const expanded = toggleCapabilityDisclosure(initial, 'chat');

    expect([...initial.expandedTasks]).toEqual([]);
    expect([...expanded.expandedTasks]).toEqual(['chat']);
    expect(expanded.expandedTasks).not.toBe(initial.expandedTasks);

    const collapsed = toggleCapabilityDisclosure(expanded, 'chat');
    expect([...expanded.expandedTasks]).toEqual(['chat']);
    expect([...collapsed.expandedTasks]).toEqual([]);
  });

  test('opens only the first new actionable error in selected-task order', () => {
    const errors: CapabilityValidationIssue[] = [
      { code: 'model_required' },
      { task: 'chat', code: 'manifest_loading' },
      { task: 'speech_synthesis', code: 'connection_missing' },
      { task: 'video_generation', code: 'invalid_provider_params' },
    ];
    const state = createCapabilityDisclosureState(
      tasks('chat', 'video_generation', 'speech_synthesis'),
      errors
    );

    expect([...state.expandedTasks]).toEqual(['video_generation']);
    expect(state.errorTasks.size).toBe(2);
    expect(state.errorTasks.has('video_generation')).toBe(true);
    expect(state.errorTasks.has('speech_synthesis')).toBe(true);
    expect(state.errorTasks.has('chat')).toBe(false);
  });

  test('does not reopen a persistent error after a user collapses it, but reopens a recurring error', () => {
    const selected = tasks('video_generation');
    const errors: CapabilityValidationIssue[] = [
      { task: 'video_generation', code: 'invalid_provider_params' },
    ];
    const automaticallyExpanded = createCapabilityDisclosureState(selected, errors);
    const manuallyCollapsed = toggleCapabilityDisclosure(automaticallyExpanded, 'video_generation');
    const persistent = syncCapabilityDisclosureState(manuallyCollapsed, selected, errors);

    expect([...persistent.expandedTasks]).toEqual([]);
    expect(persistent.errorTasks.has('video_generation')).toBe(true);

    const resolved = syncCapabilityDisclosureState(persistent, selected, []);
    const recurring = syncCapabilityDisclosureState(resolved, selected, errors);
    expect([...recurring.expandedTasks]).toEqual(['video_generation']);
  });

  test('keeps an existing error seen while another capability recommendation settles', () => {
    const chatError: CapabilityValidationIssue = {
      task: 'chat',
      code: 'invalid_provider_params',
    };
    const videoError: CapabilityValidationIssue = {
      task: 'video_generation',
      code: 'protocol_required',
    };
    const selected = tasks('chat', 'video_generation');
    const initial = createCapabilityDisclosureState(selected, [chatError]);
    const manuallyCollapsed = toggleCapabilityDisclosure(initial, 'chat');
    const whileVideoSettles = getSettledCapabilityValidationErrors(
      [chatError, videoError],
      new Set<ModelTask>(['video_generation']),
      false
    );
    const pending = syncCapabilityDisclosureState(manuallyCollapsed, selected, whileVideoSettles);
    const settled = syncCapabilityDisclosureState(pending, selected, [chatError, videoError]);

    expect([...pending.errorTasks]).toEqual(['chat']);
    expect([...settled.expandedTasks]).toEqual(['video_generation']);
    expect(settled.expandedTasks.has('chat')).toBe(false);
  });

  test('removes disclosure and error state for deselected capabilities', () => {
    const errors: CapabilityValidationIssue[] = [
      { task: 'video_generation', code: 'connection_missing' },
    ];
    const withError = createCapabilityDisclosureState(tasks('chat', 'video_generation'), errors);
    const withChatExpanded = toggleCapabilityDisclosure(withError, 'chat');
    const afterDeselection = syncCapabilityDisclosureState(withChatExpanded, tasks('chat'), errors);

    expect([...afterDeselection.expandedTasks]).toEqual(['chat']);
    expect([...afterDeselection.errorTasks]).toEqual([]);
  });
});
