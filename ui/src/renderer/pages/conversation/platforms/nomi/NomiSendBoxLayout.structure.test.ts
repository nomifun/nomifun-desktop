/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Nomi sendbox control layout', () => {
  test('renders context usage as a click ring before the model selector and removes turn metrics copy', () => {
    const source = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const sendBoxSource = readSource(new URL('../../../../components/chat/SendBox/index.tsx', import.meta.url));
    const contextRingSource = readSource(new URL('./ContextUsageRing.tsx', import.meta.url));
    const useNomiMessageSource = readSource(new URL('./useNomiMessage.ts', import.meta.url));
    const sendBoxIndex = source.indexOf('<SendBox');
    const rightToolsIndex = source.indexOf('rightTools={');
    const modelIndex = source.indexOf('<NomiModelSelector', rightToolsIndex);
    const contextRingIndex = source.indexOf('<ContextUsageRing', rightToolsIndex);
    const collaboratorIndex = source.indexOf('{collaboratorSelectorNode}', rightToolsIndex);

    expect(sendBoxIndex).toBeGreaterThan(-1);
    expect(rightToolsIndex).toBeGreaterThan(sendBoxIndex);
    expect(contextRingIndex).toBeGreaterThan(rightToolsIndex);
    expect(modelIndex).toBeGreaterThan(contextRingIndex);
    expect(collaboratorIndex).toBeGreaterThan(modelIndex);
    expect(source.includes('topRightTools={')).toBe(false);
    expect(source.includes('ContextUsagePill')).toBe(false);
    expect(source.includes("data-testid='nomi-context-usage-slot'")).toBe(false);
    expect(source.includes("data-testid='nomi-turn-metrics'")).toBe(false);
    expect(source.includes('formatTurnDuration')).toBe(false);
    expect(source.includes('formatTokenCount(tokenUsage.total_tokens)')).toBe(false);
    expect(sendBoxSource.includes("data-testid='sendbox-internal-status-row'")).toBe(true);
    expect(sendBoxSource.includes("data-testid='sendbox-top-right-tools'")).toBe(false);
    expect(contextRingSource.includes("data-testid='nomi-context-usage-ring'")).toBe(true);
    expect(contextRingSource.includes("data-testid='nomi-context-usage-popover'")).toBe(true);
    expect(contextRingSource.includes("trigger='click'")).toBe(true);
    expect(contextRingSource.includes('conic-gradient')).toBe(true);
    expect(contextRingSource.includes('h-22px w-22px')).toBe(true);
    expect(contextRingSource.includes('formatTokenCount(used)')).toBe(true);
    expect(contextRingSource.includes('formatTokenCount(max)')).toBe(true);
    expect(contextRingSource.includes('inputTokens != null || outputTokens != null || reasoningTokens != null')).toBe(
      true
    );
    expect(contextRingSource.includes('formatTokenCount(outputTokens)')).toBe(true);
    expect(contextRingSource.includes('formatTokenCount(reasoningTokens)')).toBe(true);
    expect(contextRingSource.includes('included in output')).toBe(true);
    expect(useNomiMessageSource.includes('total_tokens: (inputTokens ?? 0) + (outputTokens ?? 0)')).toBe(true);
    expect(contextRingSource.includes("data-testid='nomi-context-usage'")).toBe(false);
    expect(contextRingSource.includes('rd-999px b b-solid px-10px')).toBe(false);
  });

  test('merges collaboration models and policy into one control next to the main model', () => {
    const chatSource = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));

    const collaborationBlock = chatSource.slice(
      chatSource.indexOf('const collaborationControlNode'),
      chatSource.indexOf('const { groups: healGroups'),
    );
    expect(collaborationBlock.includes('<GuidCollaboratorSelector')).toBe(true);
    expect(collaborationBlock.includes('onChange={onCollaboratorsChange}')).toBe(true);
    expect(collaborationBlock.includes('panelFooter={')).toBe(true);
    expect(collaborationBlock.includes('<CollaborationPolicyControl')).toBe(true);
    expect(collaborationBlock.includes('onChange={onCollaborationPolicyChange}')).toBe(true);
    expect(collaborationBlock.includes('embedded')).toBe(true);
    expect(collaborationBlock.includes("triggerLabel={t('collaboration.policy.button'")).toBe(true);
    expect(collaborationBlock.includes("className='nomi-sendbox-model-btn nomi-sendbox-collaboration-btn'")).toBe(true);
    expect(chatSource.includes('extraRightTools={collaborationPolicyNode}')).toBe(false);

    const rightToolsIndex = sendBoxSource.indexOf('rightTools={');
    const contextRingIndex = sendBoxSource.indexOf('<ContextUsageRing', rightToolsIndex);
    const modelIndex = sendBoxSource.indexOf('<NomiModelSelector', rightToolsIndex);
    const collaboratorIndex = sendBoxSource.indexOf('{collaboratorSelectorNode}', rightToolsIndex);
    const permissionIndex = sendBoxSource.indexOf('<AgentModeSelector', rightToolsIndex);

    expect(contextRingIndex).toBeGreaterThan(rightToolsIndex);
    expect(modelIndex).toBeGreaterThan(contextRingIndex);
    expect(collaboratorIndex).toBeGreaterThan(modelIndex);
    expect(permissionIndex).toBeGreaterThan(collaboratorIndex);
  });

  test('reconciles conversation collaborators before rendering or persisting executable ranges', () => {
    const chatSource = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));

    expect(chatSource.includes('import { reconcileModelRefs, sameModelRefs }')).toBe(true);
    expect(chatSource.includes('const activeCollaborators = collaboratorReconciliation?.active ?? []')).toBe(true);
    expect(chatSource.includes('value={activeCollaborators}')).toBe(true);
    expect(
      /buildConversationModelPool\(\s*\{ provider_id: _provider\.id, model: modelName \},\s*activeCollaborators,\s*\)/.test(
        chatSource,
      ),
    ).toBe(true);
    expect(chatSource.includes('collaboratorReconciliation.removed.length === 0')).toBe(true);
    expect(chatSource.includes('sameModelRefs(collaborators, collaboratorReconciliation.retained)')).toBe(true);
  });

  test('supports embedding the policy panel behind the unified collaboration trigger', () => {
    const source = readSource(
      new URL('../../../../components/collaboration/CollaborationPolicyControl.tsx', import.meta.url),
    );

    expect(source.includes("data-testid='collaboration-policy-control'")).toBe(true);
    expect(source.includes('embedded?: boolean')).toBe(true);
    expect(source.includes('if (embedded)')).toBe(true);
    expect(source.includes('return <div className={styles.embedded}>{content}</div>')).toBe(true);
  });

  test('collapses text pills to icons and expands their labels inline on desktop hover', () => {
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const modelSource = readSource(new URL('./NomiModelSelector.tsx', import.meta.url));
    const sendBoxCss = readSource(new URL('../../../../components/chat/SendBox/sendbox.css', import.meta.url));
    const collaboratorSource = readSource(new URL('../../../guid/components/GuidCollaboratorSelector.tsx', import.meta.url));
    const modeSource = readSource(new URL('../../../../components/agent/AgentModeSelector.tsx', import.meta.url));
    const summonSource = readSource(new URL('../../components/SummonPanel/index.tsx', import.meta.url));

    expect(sendBoxSource.includes('sendbox-responsive-config-group')).toBe(true);
    expect(sendBoxCss.includes('container-name: sendbox-config')).toBe(true);
    expect(sendBoxCss.includes('@container sendbox-config (max-width: 560px)')).toBe(true);
    expect(sendBoxCss.includes('.sendbox-responsive-label')).toBe(true);
    expect(sendBoxCss.includes(".nomi-sendbox-collaboration-btn[aria-pressed='true']")).toBe(true);
    expect(sendBoxCss.includes('max-width 160ms ease')).toBe(true);
    expect(sendBoxCss.includes('@media (hover: hover) and (pointer: fine)')).toBe(true);
    expect(sendBoxCss.includes('.nomi-sendbox-model-btn:hover')).toBe(true);
    expect(sendBoxCss.includes('display: inline-flex !important')).toBe(true);

    for (const source of [modelSource, collaboratorSource, modeSource]) {
      expect(source.includes('<Tooltip')).toBe(false);
      expect(source.includes('sendbox-responsive-label')).toBe(true);
      expect(source.includes('aria-label=')).toBe(true);
    }
    const summonControlSource = summonSource.slice(summonSource.indexOf('const SummonControl'));
    expect(summonControlSource.includes('<Tooltip')).toBe(false);
    expect(summonSource.includes('sendbox-responsive-label')).toBe(true);
    expect(summonSource.includes("className='nomi-sendbox-summon-btn'")).toBe(true);
  });
});
