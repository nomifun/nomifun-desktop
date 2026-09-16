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
    const sendBoxSource = readSource(new URL('../../../../components/chat/Composer.tsx', import.meta.url));
    const contextRingSource = readSource(new URL('./ContextUsageRing.tsx', import.meta.url));
    const useNomiMessageSource = readSource(new URL('./useNomiMessage.ts', import.meta.url));
    const sendBoxIndex = source.indexOf('<SendBox');
    const rightToolsIndex = source.indexOf('rightTools={');
    const modelIndex = source.indexOf('<NomiModelSelector', rightToolsIndex);
    const contextRingIndex = source.indexOf('<ContextUsageRing', rightToolsIndex);
    const sideToolsIndex = source.indexOf('sideTools={');
    const collaboratorIndex = source.indexOf('{collaboratorSelectorNode}', sideToolsIndex);

    expect(sendBoxIndex).toBeGreaterThan(-1);
    expect(rightToolsIndex).toBeGreaterThan(sendBoxIndex);
    expect(contextRingIndex).toBeGreaterThan(rightToolsIndex);
    expect(modelIndex).toBeGreaterThan(contextRingIndex);
    expect(collaboratorIndex).toBeGreaterThan(sideToolsIndex);
    expect(collaboratorIndex).toBeLessThan(rightToolsIndex);
    expect(source.includes('topRightTools=')).toBe(false);
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

  test('keeps collaboration models and policy together in the side rail', () => {
    const chatSource = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));

    const collaborationBlock = chatSource.slice(
      chatSource.indexOf('const collaborationControlNode'),
      chatSource.indexOf('const { groups: healGroups'),
    );
    expect(collaborationBlock.includes('<CollaborationComposerControl')).toBe(true);
    const sharedControl = readSource(new URL('../../../../components/collaboration/CollaborationComposerControl.tsx', import.meta.url));
    const homeSource = readSource(new URL('../../../guid/GuidPage.tsx', import.meta.url));
    expect(homeSource.includes('<CollaborationComposerControl')).toBe(true);
    expect(homeSource.includes('collaboration: collaboration.config')).toBe(true);
    expect(collaborationBlock.includes('onChange={onCollaboratorsChange}')).toBe(true);
    expect(sharedControl.includes('panelFooter={')).toBe(true);
    expect(sharedControl.includes('<CollaborationPolicyControl')).toBe(true);
    expect(collaborationBlock.includes('onPolicyChange={onCollaborationPolicyChange}')).toBe(true);
    expect(sharedControl.includes('embedded')).toBe(true);
    expect(sharedControl.includes("triggerLabel={t('collaboration.policy.button'")).toBe(true);
    expect(sharedControl.includes("className='nomi-sendbox-model-btn nomi-sendbox-collaboration-btn'")).toBe(true);
    expect(chatSource.includes('extraRightTools={collaborationPolicyNode}')).toBe(false);

    const rightToolsIndex = sendBoxSource.indexOf('rightTools={');
    const contextRingIndex = sendBoxSource.indexOf('<ContextUsageRing', rightToolsIndex);
    const modelIndex = sendBoxSource.indexOf('<NomiModelSelector', rightToolsIndex);
    const sideToolsIndex = sendBoxSource.indexOf('sideTools={');
    const collaboratorIndex = sendBoxSource.indexOf('{collaboratorSelectorNode}', sideToolsIndex);

    expect(contextRingIndex).toBeGreaterThan(rightToolsIndex);
    expect(modelIndex).toBeGreaterThan(contextRingIndex);
    expect(collaboratorIndex).toBeGreaterThan(sideToolsIndex);
    expect(collaboratorIndex).toBeLessThan(rightToolsIndex);
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

  test('allows session model selection while keeping preset resource restrictions', () => {
    const chatSource = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));
    const nomiChatSource = readSource(new URL('./NomiChat.tsx', import.meta.url));
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const selectorSource = readSource(new URL('../../../../components/chat/ChatModelSelector.tsx', import.meta.url));

    expect(chatSource.includes('modelLocked')).toBe(false);
    expect(chatSource.includes('const hasPreset = Boolean(conversation.preset_id);')).toBe(true);
    expect(chatSource.includes('useAgentCapabilityResourceKinds')).toBe(false);
    expect(chatSource.includes('required_resource_kinds')).toBe(true);

    expect(nomiChatSource.includes('modelLocked')).toBe(false);
    expect(sendBoxSource.includes('hideAdvancedControls || modelLocked')).toBe(false);
    expect(sendBoxSource.includes('{!modelLocked && (')).toBe(false);
    expect(sendBoxSource.includes('modelLocked')).toBe(false);
    expect(sendBoxSource.includes('<NomiModelSelector')).toBe(true);
    expect(sendBoxSource.includes('{collaboratorSelectorNode}')).toBe(true);
    expect(selectorSource.includes('if (disabled) return trigger;')).toBe(true);
    expect(selectorSource.includes("data-readonly={disabled ? 'true' : undefined}")).toBe(true);
  });

  test('exposes the shared Agent catalog in conversation controls', () => {
    const chatSource = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));
    const nomiChatSource = readSource(new URL('./NomiChat.tsx', import.meta.url));
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const agentSwitchBlock = chatSource.slice(
      chatSource.indexOf('const switchAgent'),
      chatSource.indexOf('const currentAgentLabel'),
    );

    expect(chatSource.includes('<GuidAgentSelector')).toBe(true);
    expect(chatSource.includes('useAgentPresets()')).toBe(true);
    expect(agentSwitchBlock.includes('sessions.switchPreset.invoke')).toBe(false);
    expect(agentSwitchBlock.includes('const resolvePreset')).toBe(true);
    expect(agentSwitchBlock.includes('setAgentChoice(selection)')).toBe(true);
    expect(sendBoxSource.includes('preset_id: presetId')).toBe(true);
    expect(agentSwitchBlock.includes('conversation.stop.invoke')).toBe(false);
    expect(nomiChatSource.includes('agentSelectorNode={agentSelectorNode}')).toBe(true);
    expect(sendBoxSource.includes('prefix={<ComposerSceneHeader agent={agentSelectorNode} />}')).toBe(true);
  });

  test('waits for passive runtime warmup before delivering the Guid initial message', () => {
    const source = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const initialMessageBlock = source.slice(
      source.indexOf('// Handle the Guid handoff only after passive warmup'),
      source.indexOf('const onSendHandler'),
    );

    expect(initialMessageBlock.includes('!agentWarmed')).toBe(true);
    expect(initialMessageBlock.includes('agentWarmed, conversation_id')).toBe(true);
    expect(initialMessageBlock.includes('initialOnly: true')).toBe(true);
  });

  test('only personal Agent selections may reuse the staged preset without preparing the current official template', () => {
    const source = readSource(new URL('../../components/ChatConversation.tsx', import.meta.url));
    const admission = source.slice(source.indexOf('const resolvePreset = async () =>'), source.indexOf('const selectCreationMode ='));
    expect(admission.includes("if (agentChoice.kind === 'preset' && stagedPresetId) return stagedPresetId;")).toBe(true);
    expect(admission.includes('if (stagedPresetId) return stagedPresetId;')).toBe(false);
    expect(admission.includes('await resolveAgentSwitchTarget(selected)')).toBe(true);
    const resolution = source.slice(source.indexOf('const resolveAgentSwitchTarget ='), source.indexOf('// Menus edit only the next draft.'));
    expect(resolution.includes("if (selection.kind === 'template')")).toBe(true);
    expect(resolution.includes('await prepareOfficialAgent(')).toBe(true);
  });

  test('collapses text pills to icons and expands their labels inline on desktop hover', () => {
    const sendBoxSource = readSource(new URL('./NomiSendBox.tsx', import.meta.url));
    const modelSource = readSource(new URL('../../../../components/chat/ChatModelSelector.tsx', import.meta.url));
    const sendBoxCss = readSource(new URL('../../../../components/chat/SendBox/sendbox.css', import.meta.url));
    const responsiveCss = readSource(new URL('../../../../components/chat/ResponsiveComposerRow.module.css', import.meta.url));
    const collaboratorSource = readSource(new URL('../../../guid/components/GuidCollaboratorSelector.tsx', import.meta.url));

    expect(sendBoxSource.includes('sendbox-responsive-config-group')).toBe(true);
    expect(sendBoxCss.includes('container-name: sendbox-config')).toBe(false);
    expect(responsiveCss.includes("[data-compact='true']")).toBe(true);
    expect(responsiveCss.includes('.sendbox-responsive-label')).toBe(true);
    expect(sendBoxCss.includes(".nomi-sendbox-collaboration-btn[aria-pressed='true']")).toBe(true);
    expect(responsiveCss.includes(':hover, :focus-visible')).toBe(true);
    expect(responsiveCss.includes('display: inline-flex !important')).toBe(true);

    for (const source of [modelSource, collaboratorSource]) {
      expect(source.includes('<Tooltip')).toBe(false);
      expect(source.includes('sendbox-responsive-label')).toBe(true);
      expect(source.includes('aria-label=')).toBe(true);
    }
  });
});
