/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { CapabilityValidationResult } from './providerModelAdvanced';

export type CapabilityValidationIssue = CapabilityValidationResult['errors'][number];

export interface CapabilityDisclosureState {
  readonly expandedTasks: ReadonlySet<ModelTask>;
  /** Current actionable errors; persistent entries are treated as already seen. */
  readonly errorTasks: ReadonlySet<ModelTask>;
}

/** Keep capability summaries useful without exposing URL paths, credentials, or query parameters. */
export const compactCapabilityUrlSummary = (value: string): string => {
  const normalized = value.trim();
  if (!normalized) return '';

  try {
    const parsed = new URL(normalized);
    if (parsed.origin === 'null') return '';
    return parsed.origin.length > 64 ? `${parsed.origin.slice(0, 61)}...` : parsed.origin;
  } catch {
    const originMatch = normalized.match(/^([a-z][a-z\d+.-]*:\/\/)(?:[^/@]+@)?([^/?#]+)/i);
    if (!originMatch) return '';
    const origin = `${originMatch[1]}${originMatch[2]}`;
    return origin.length > 64 ? `${origin.slice(0, 61)}...` : origin;
  }
};

export const getActionableCapabilityErrorTasks = (
  errors: readonly CapabilityValidationIssue[]
): Set<ModelTask> => {
  const tasks = new Set<ModelTask>();
  for (const error of errors) {
    if (error.task && error.code !== 'manifest_loading') tasks.add(error.task);
  }
  return tasks;
};

/** Suppress only errors that can be transient while dependencies are settling. */
export const getSettledCapabilityValidationErrors = (
  errors: readonly CapabilityValidationIssue[],
  recommendationPendingTasks: ReadonlySet<ModelTask>,
  validationPending: boolean
): CapabilityValidationIssue[] =>
  errors.filter(
    (error) =>
      (!error.task || !recommendationPendingTasks.has(error.task)) &&
      !(validationPending && error.code === 'connection_missing')
  );

export const syncCapabilityDisclosureState = (
  state: CapabilityDisclosureState,
  selectedTasks: readonly ModelTask[],
  errors: readonly CapabilityValidationIssue[]
): CapabilityDisclosureState => {
  const selectedTaskSet = new Set(selectedTasks);
  const actionableErrorTasks = getActionableCapabilityErrorTasks(errors);
  const errorTasks = new Set<ModelTask>(
    [...actionableErrorTasks].filter((task) => selectedTaskSet.has(task))
  );
  const expandedTasks = new Set<ModelTask>(
    [...state.expandedTasks].filter((task) => selectedTaskSet.has(task))
  );

  const firstNewErrorTask = selectedTasks.find(
    (task) => errorTasks.has(task) && !state.errorTasks.has(task)
  );
  if (firstNewErrorTask) expandedTasks.add(firstNewErrorTask);

  return { expandedTasks, errorTasks };
};

export const createCapabilityDisclosureState = (
  selectedTasks: readonly ModelTask[],
  errors: readonly CapabilityValidationIssue[] = []
): CapabilityDisclosureState =>
  syncCapabilityDisclosureState(
    {
      expandedTasks: new Set<ModelTask>(),
      errorTasks: new Set<ModelTask>(),
    },
    selectedTasks,
    errors
  );

export const toggleCapabilityDisclosure = (
  state: CapabilityDisclosureState,
  task: ModelTask
): CapabilityDisclosureState => {
  const expandedTasks = new Set(state.expandedTasks);
  if (expandedTasks.has(task)) expandedTasks.delete(task);
  else expandedTasks.add(task);

  return {
    expandedTasks,
    errorTasks: new Set(state.errorTasks),
  };
};
