/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary, OfficialPresetKey, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { AgentPresetId } from '@/common/types/ids';
import { autoUpdate, flip, FloatingFocusManager, FloatingPortal, offset, shift, size, useClick, useDismiss, useFloating, useInteractions, useRole } from '@floating-ui/react';
import { Check, Code, Down, Edit, Magic, MessageOne, Right, Robot, Search, User } from '@icon-park/react';
import React, { useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useNavigate } from 'react-router-dom';
import { TEMPLATE_I18N_PATH } from '../../agentSettings/model';
import type { ExecutableAgentPreset, GuidAgentSelection } from '../types';
import { isConversationAgentTemplate } from '@/renderer/components/agent/conversationAgentCatalog';
import styles from './GuidAgentSelector.module.css';

export const DEFAULT_VISIBLE_PERSONAL_AGENTS = 3;

export type GuidAgentSelectorProps = {
  presets: ExecutableAgentPreset[];
  draftPresets?: AgentPresetSummary[];
  officialTemplates?: OfficialPresetTemplate[];
  selection: GuidAgentSelection;
  isLoading?: boolean;
  loadError?: Error;
  onRetry?: () => Promise<void>;
  selectedLabelOverride?: string;
  disabled?: boolean;
  onSelectTemplate: (templateKey: OfficialPresetKey) => void;
  onSelectPreset: (presetId: AgentPresetId) => void;
};

const templateIcon = (key: OfficialPresetKey) => {
  const Icon = key === 'coding.codex' ? Code
    : key === 'chat.minimal' ? MessageOne
      : key === 'creative-studio.default' ? Magic
        : User;
  return <Icon theme='outline' size={20} fill='currentColor' />;
};

const GuidAgentSelector: React.FC<GuidAgentSelectorProps> = ({
  presets,
  draftPresets = [],
  officialTemplates = [],
  selection,
  isLoading = false,
  loadError,
  onRetry,
  selectedLabelOverride,
  disabled = false,
  onSelectTemplate,
  onSelectPreset,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [allPersonalAgents, setAllPersonalAgents] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const mineHeading = useId();
  const templateHeading = useId();
  const selectedPreset = selection.kind === 'preset'
    ? presets.find((preset) => preset.preset_id === selection.presetId)
    : undefined;
  const selectedLabel = selectedLabelOverride ?? (selection.kind === 'template'
    ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.templateKey]}.name`)
    : selectedPreset?.display_name ?? t('guid.agentEntries.choose'));

  const changeOpen = (next: boolean) => {
    setOpen(next);
    if (!next) {
      setQuery('');
      setAllPersonalAgents(false);
    }
  };
  const { refs, floatingStyles, context } = useFloating({
    open,
    onOpenChange: changeOpen,
    placement: 'bottom-start',
    strategy: 'fixed',
    whileElementsMounted: autoUpdate,
    middleware: [
      offset(8),
      flip({ padding: 12, fallbackStrategy: 'initialPlacement' }),
      shift({ padding: 12 }),
      size({
        padding: 12,
        apply({ availableHeight, elements }) {
          elements.floating.style.maxHeight = `${Math.max(0, Math.min(520, availableHeight))}px`;
        },
      }),
    ],
  });
  const click = useClick(context);
  const dismiss = useDismiss(context);
  const role = useRole(context, { role: 'dialog' });
  const { getReferenceProps, getFloatingProps } = useInteractions([click, dismiss, role]);

  const normalizedQuery = query.trim().toLocaleLowerCase();
  const matches = (name: string, description = '') =>
    `${name} ${description}`.toLocaleLowerCase().includes(normalizedQuery);
  const savedMatches = presets.filter((preset) => matches(preset.display_name, preset.description));
  const draftMatches = draftPresets.filter((preset) => matches(preset.display_name, preset.description));
  const templates = useMemo(() => officialTemplates
    .filter(isConversationAgentTemplate)
    .map((template) => ({
      ...template,
      name: t(`agentSettings.template.${TEMPLATE_I18N_PATH[template.template_key]}.name`),
      description: t(`agentSettings.template.${TEMPLATE_I18N_PATH[template.template_key]}.description`),
    })), [officialTemplates, t]);
  const templateMatches = templates.filter((template) => matches(template.name, template.description));
  const mineMatches = [
    ...savedMatches.map((preset) => ({ kind: 'saved' as const, preset })),
    ...draftMatches.map((preset) => ({ kind: 'draft' as const, preset })),
  ];
  const collapsedMineMatches = mineMatches.slice(0, DEFAULT_VISIBLE_PERSONAL_AGENTS);
  if (selection.kind === 'preset') {
    const selectedIndex = mineMatches.findIndex((entry) => entry.preset.preset_id === selection.presetId);
    if (selectedIndex >= DEFAULT_VISIBLE_PERSONAL_AGENTS) {
      collapsedMineMatches.splice(
        DEFAULT_VISIBLE_PERSONAL_AGENTS - 1,
        1,
        mineMatches[selectedIndex]
      );
    }
  }
  const visibleMineMatches = normalizedQuery || allPersonalAgents
    ? mineMatches
    : collapsedMineMatches;
  const hiddenPersonalCount = Math.max(0, mineMatches.length - collapsedMineMatches.length);
  const hasMine = mineMatches.length > 0;
  const noMatches = !hasMine && templateMatches.length === 0;

  const choose = (action: () => void) => {
    changeOpen(false);
    action();
  };
  const onMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
    const choices = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[data-agent-choice]'));
    if (event.key === 'Enter' && event.target === searchRef.current) {
      if (choices[0]) {
        event.preventDefault();
        choices[0].click();
      }
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const current = choices.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'ArrowDown'
      ? (current + 1) % choices.length
      : current <= 0 ? choices.length - 1 : current - 1;
    choices[next]?.focus();
    choices[next]?.scrollIntoView?.({ block: 'nearest' });
  };

  return (
    <>
      <button
        ref={refs.setReference}
        type='button'
        className={styles.trigger}
        disabled={disabled}
        title={selectedLabel}
        aria-label={selectedLabel}
        data-testid='guid-agent-selector'
        {...getReferenceProps()}
      >
        <Robot theme='outline' size={18} fill='currentColor' />
        <span className={styles.triggerLabel}>{selectedLabel}</span>
        <Down theme='outline' size={12} fill='currentColor' className={open ? styles.chevronOpen : undefined} />
      </button>
      {open && (
        <FloatingPortal>
          <FloatingFocusManager context={context} initialFocus={searchRef} modal={false}>
            <div
              ref={refs.setFloating}
              style={floatingStyles}
              className={styles.panel}
              aria-label={t('guid.agentEntries.choose')}
              {...getFloatingProps({ onKeyDown: onMenuKeyDown })}
            >
              <div className={styles.searchWrap}>
                <Search theme='outline' size={18} fill='currentColor' />
                <input
                  ref={searchRef}
                  type='search'
                  value={query}
                  onInput={(event) => setQuery(event.currentTarget.value)}
                  placeholder={t('guid.agentEntries.search')}
                  aria-label={t('guid.agentEntries.search')}
                  className={styles.searchInput}
                />
              </div>
              <div className={styles.results}>
                {isLoading && <p className={styles.notice} role='status'>{t('guid.agentEntries.loading')}</p>}
                {loadError && (
                  <div className={styles.notice} role='alert'>
                    {t('guid.agentEntries.loadFailed')}
                    {onRetry && <button type='button' className={styles.textAction} onClick={() => void onRetry()}>{t('agentSettings.actions.retry')}</button>}
                  </div>
                )}
                {templateMatches.length > 0 && (
                  <section role='group' aria-labelledby={templateHeading}>
                    <h3 id={templateHeading} className={styles.heading}>{t('guid.agentEntries.fromTemplate')}</h3>
                    <div className={styles.templateGrid}>
                      {templateMatches.map((template) => (
                        <AgentRow key={template.template_key} name={template.name} icon={templateIcon(template.template_key)} selected={selection.kind === 'template' && selection.templateKey === template.template_key} onClick={() => choose(() => onSelectTemplate(template.template_key))} />
                      ))}
                    </div>
                  </section>
                )}
                {hasMine && (
                  <section role='group' aria-labelledby={mineHeading} className={templateMatches.length > 0 ? styles.personalSection : undefined}>
                    <h3 id={mineHeading} className={styles.heading}>{t('agentSettings.library.mine')}</h3>
                    {visibleMineMatches.map(({ kind, preset }) => kind === 'saved' ? (
                      <AgentRow key={`saved:${preset.preset_id}`} name={preset.display_name} description={preset.description} icon={<User theme='outline' size={21} fill='currentColor' />} selected={selection.kind === 'preset' && selection.presetId === preset.preset_id} onClick={() => choose(() => onSelectPreset(preset.preset_id))} />
                    ) : (
                      <AgentRow key={`draft:${preset.preset_id}`} name={preset.display_name} description={preset.description} icon={<Edit theme='outline' size={20} fill='currentColor' />} status={t('guid.agentEntries.needsSetup')} onClick={() => choose(() => navigate(`/agent?preset=${encodeURIComponent(preset.preset_id)}`))} />
                    ))}
                    {!normalizedQuery && mineMatches.length > DEFAULT_VISIBLE_PERSONAL_AGENTS && (
                      <button
                        type='button'
                        className={styles.moreButton}
                        aria-expanded={allPersonalAgents}
                        onClick={() => setAllPersonalAgents((current) => !current)}
                      >
                        {t(allPersonalAgents
                          ? 'guid.agentEntries.showFewerPersonal'
                          : 'guid.agentEntries.showMorePersonal', { count: hiddenPersonalCount })}
                      </button>
                    )}
                  </section>
                )}
                {!isLoading && noMatches && <p className={styles.empty} role='status'>{t('guid.agentEntries.empty')}</p>}
              </div>
              <footer className={styles.footer}>
                <Link className={styles.manageLink} to='/agent' onClick={() => changeOpen(false)}>{t('guid.agentEntries.manage')}</Link>
              </footer>
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </>
  );
};

const AgentRow: React.FC<{
  name: string;
  description?: string;
  icon: React.ReactNode;
  selected?: boolean;
  status?: string;
  onClick: () => void;
}> = ({ name, description, icon, selected, status, onClick }) => (
  <button type='button' data-agent-choice className={`${styles.row} ${selected ? styles.selected : ''}`} aria-label={name} aria-pressed={selected} onClick={onClick}>
    <span className={styles.rowIcon} aria-hidden='true'>{icon}</span>
    <span className={styles.rowCopy}>
      <span className={styles.rowName}>{name}</span>
      {description && <span className={styles.rowDescription}>{description}</span>}
    </span>
    {status && <span className={styles.rowStatus}>{status}</span>}
    {selected ? <Check className={styles.check} theme='outline' size={18} fill='currentColor' />
      : selected === undefined ? <Right className={styles.arrow} theme='outline' size={15} fill='currentColor' /> : null}
  </button>
);

export default GuidAgentSelector;
