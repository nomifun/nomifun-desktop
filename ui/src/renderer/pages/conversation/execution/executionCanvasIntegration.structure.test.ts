/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('conversation execution canvas integration', () => {
  test('keeps execution progress native to the collaboration pane', () => {
    const chatSource = readSource(new URL('../components/ChatConversation.tsx', import.meta.url));
    const layoutSource = readSource(new URL('./ExecutionConversationLayout.tsx', import.meta.url));
    const panelSource = readSource(new URL('./ExecutionTopPanel.tsx', import.meta.url));

    expect(chatSource.includes('<ExecutionProvider')).toBe(true);
    expect(chatSource.includes('<ExecutionConversationLayout')).toBe(true);
    expect(layoutSource.includes('<ExecutionTopPanel')).toBe(true);
    expect(layoutSource.includes("className='flex-1 min-w-0 min-h-0 flex flex-col'")).toBe(true);

    expect(panelSource.includes("data-testid='execution-canvas-progress'")).toBe(true);
    expect(panelSource.includes('stepStatusMeta(step.status)')).toBe(true);
    expect(panelSource.includes('projectStep({')).toBe(true);
  });

  test('renders only the current plan revision while retaining history in the detail model', () => {
    const canvasSource = readSource(new URL('./DagCanvas.tsx', import.meta.url));
    const typesSource = readSource(new URL('../../../../common/types/agentExecution/agentExecutionTypes.ts', import.meta.url));

    expect(canvasSource.includes('step.superseded_in_revision == null')).toBe(true);
    expect(canvasSource.includes('dependency.superseded_in_revision == null')).toBe(true);
    expect(typesSource.includes('introduced_in_revision: number')).toBe(true);
    expect(typesSource.includes('superseded_in_revision: number | null')).toBe(true);
    expect(typesSource.includes('attempts: TExecutionAttempt[]')).toBe(true);
  });

  test('keeps collaboration controls aligned with execution state and immutable step replacement', () => {
    const controlsSource = readSource(new URL('./ExecutionControls.tsx', import.meta.url));
    const projectedSource = readSource(new URL('./ProjectedAttemptView.tsx', import.meta.url));

    expect(controlsSource.includes("status === 'running' || status === 'waiting_input'")).toBe(true);
    expect(controlsSource.includes("status !== '' && !isBusyPlaceholder && !isTerminal")).toBe(true);
    expect(projectedSource.includes("step.kind === 'agent' && step.status === 'pending'")).toBe(true);
    expect(projectedSource.includes('participant.retired_in_revision == null')).toBe(true);
    expect(projectedSource.includes('projectReplacementStep(replacement)')).toBe(true);
    expect(projectedSource.includes('projectionKey: payload.projectionKey ?? payload.step.id')).toBe(true);
    expect(projectedSource.includes('canSteerExecutionAttempt(attempt?.status, detail?.execution.status)')).toBe(true);
    expect(projectedSource.includes('ipcBridge.agentExecution.steer.invoke')).toBe(true);
    expect(projectedSource.includes('expected_execution_version: detail.execution.version')).toBe(true);
  });

  test('keeps the collaboration panel recoverable and usable on compact layouts', () => {
    const layoutSource = readSource(new URL('./ExecutionConversationLayout.tsx', import.meta.url));
    const panelCss = readSource(new URL('./executionTopPanel.module.css', import.meta.url));

    expect(layoutSource.includes('execution.toggleCanvas')).toBe(true);
    expect(layoutSource.includes("'agentExecution.panel.open'")).toBe(true);
    expect(panelCss.includes('@media (max-width: 768px)')).toBe(true);
    expect(panelCss.includes('width: 100% !important')).toBe(true);
  });

  test('projects linked executions for every conversation runtime and companion sessions', () => {
    const chatSource = readSource(new URL('../components/ChatConversation.tsx', import.meta.url));
    const companionSource = readSource(
      new URL('../../nomi/companion/CompanionConversation.tsx', import.meta.url),
    );
    const companionPanelSource = readSource(
      new URL('../../nomi/companion/CompanionChatPanel.tsx', import.meta.url),
    );
    const hookSource = readSource(new URL('./useConversationExecution.ts', import.meta.url));
    const readOnlySource = readSource(new URL('./ReadOnlyConversationView.tsx', import.meta.url));

    expect(chatSource.match(/<ExecutionProvider conversation=\{conversation\}>/g)?.length).toBeGreaterThanOrEqual(3);
    expect(companionSource.includes('<ExecutionConversationLayout')).toBe(false);
    expect(companionPanelSource.includes('renderInExecutionShell')).toBe(true);
    expect(companionPanelSource.includes('<ExecutionConversationLayout')).toBe(true);
    expect(hookSource.includes("conversation?.type === 'nomi'")).toBe(false);
    expect(hookSource.includes('agentExecution.events.changed.on')).toBe(true);
    expect(hookSource.includes('getConversationOrNull(conversationId)')).toBe(true);
    expect(chatSource.includes('isRetainedAttemptTranscript')).toBe(true);
    expect(chatSource.includes('<ReadOnlyConversationView')).toBe(true);
    expect(readOnlySource.match(/hideSendBox/g)?.length).toBeGreaterThanOrEqual(6);
    expect(readOnlySource.match(/readOnly/g)?.length).toBeGreaterThanOrEqual(6);
    expect(readOnlySource.includes('ipcBridge.conversation.update')).toBe(false);
  });

  test('preserves conversational plan adjustment on the unified execution surface', () => {
    const panelSource = readSource(new URL('./ExecutionTopPanel.tsx', import.meta.url));
    const adjustSource = readSource(new URL('./ExecutionAdjustBox.tsx', import.meta.url));

    expect(panelSource.includes('<ExecutionAdjustBox')).toBe(true);
    expect(adjustSource.includes('ipcBridge.agentExecution.adjust.invoke')).toBe(true);
    expect(adjustSource.includes('expected_version: detail.execution.version')).toBe(true);
    expect(adjustSource.includes('summarizeAdjustment(detail, next)')).toBe(true);
  });
});
