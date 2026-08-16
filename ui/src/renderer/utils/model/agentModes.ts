/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Agent mode option interface
 * 代理模式选项接口
 */
export interface AgentModeOption {
  /** Mode value sent to agent / 发送给代理的模式值 */
  value: string;
  /** Display label matching CLI display / 与 CLI 显示一致的标签 */
  label: string;
  /** Optional description / 可选描述 */
  description?: string;
}

/**
 * Agent modes configuration.
 * Maps backend type to available modes.
 * Labels match the runtime display text exactly — no i18n.
 */
export const AGENT_MODES: Record<string, AgentModeOption[]> = {
  nomi: [
    { value: 'default', label: 'Default' },
    { value: 'auto_edit', label: 'Auto-Accept Edits' },
    { value: 'yolo', label: 'YOLO' },
  ],
};

/**
 * Get available modes for a given backend
 * Returns empty array if backend doesn't support mode switching
 *
 * @param backend - Agent backend type
 * @returns Array of available modes
 */
export function getAgentModes(backend: string | undefined): AgentModeOption[] {
  if (!backend) return [];
  return AGENT_MODES[backend] || [];
}

/**
 * Convert a snake_case mode value to a title-cased label.
 * e.g. 'auto_edit' -> 'Auto Edit', 'plan' -> 'Plan'
 */
function toTitleCase(value: string): string {
  return value
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * Merge static mode definitions with dynamic capabilities from the agent.
 * - If capabilityModes is null/empty, return static modes (fallback).
 * - Otherwise, return only modes reported by capabilities, preserving
 *   static labels when available and title-casing unknown modes.
 *
 * @param backend - Agent backend type
 * @param capabilityModes - Dynamic modes from capabilities.modes (null = not available)
 */
export function mergeWithCapabilities(
  backend: string | undefined,
  capabilityModes: string[] | null
): AgentModeOption[] {
  const staticModes = getAgentModes(backend);
  if (!capabilityModes || capabilityModes.length === 0) {
    return staticModes;
  }

  const staticMap = new Map(staticModes.map((m) => [m.value, m]));
  return capabilityModes.map((value) => staticMap.get(value) ?? { value, label: toTitleCase(value) });
}

/**
 * Check if a backend supports mode switching during session
 *
 * @param backend - Agent backend type
 * @returns true if mode switching is supported
 */
export function supportsModeSwitch(backend: string | undefined): boolean {
  if (!backend) return false;
  return backend in AGENT_MODES && AGENT_MODES[backend].length > 0;
}

/**
 * Full-auto mode value per backend.
 * Re-exported from common for backward compatibility.
 */
export { getFullAutoMode } from '@/common/types/agent/agentModes';
