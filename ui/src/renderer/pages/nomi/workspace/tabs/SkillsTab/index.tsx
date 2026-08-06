/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Modal, Spin } from '@arco-design/web-react';
import { NomiSettingList, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import ContentAside from '@/renderer/components/layout/ContentAside';
import { resolveSkillDisplay } from '@/renderer/pages/settings/skill/skillDisplay';
import { toggleCompanionSkill } from './companionSkillConfig';
import { useAsidePortal } from '../../AsideHost';
import type { WorkspaceTabProps } from '../../types';
import CatalogPickerModal from './CatalogPickerModal';
import CatalogSkillDetail from './CatalogSkillDetail';
import GeneratedSkillDetail from './GeneratedSkillDetail';
import LearnFromSessionModal from './LearnFromSessionModal';
import SkillListRow from './SkillListRow';
import SkillsEmptyState from './SkillsEmptyState';
import SkillsToolbar from './SkillsToolbar';
import { useSkillsTabData } from './useSkillsTabData';
import {
  buildSkillEntries,
  countDrafts,
  filterSkillEntries,
  EMPTY_SKILL_CONFIG,
  type CatalogSkillInfo,
  type SkillEntry,
  type SkillSourceFilter,
} from './unify';

/** Stable empty set for the render pass before the profile has arrived. */
const NO_AUTO_NAMES: ReadonlySet<string> = new Set<string>();

/**
 * 技能 Tab — ONE list of the skills this companion has.
 *
 * Two mechanisms feed that list (capabilities granted from the Skill library,
 * and skills the companion mined out of real work); a source badge is the only
 * place that distinction shows. Drafts wait on the user, so they sort first and
 * are this tab's attention signal. Nothing here is shared with another
 * companion: the registry list is scoped to it by companion_id.
 */
const SkillsTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t, i18n } = useTranslation();
  const data = useSkillsTabData(companionId);
  const [filter, setFilter] = useState<SkillSourceFilter>('all');
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  // Edit mode lives HERE, not as a "start in edit" hint the detail pane latches
  // once: a hint makes the row's 编辑 button dead after the first 取消 (the flag
  // is already true, so nothing changes and the pane never reopens the editor).
  const [editing, setEditing] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [learnOpen, setLearnOpen] = useState(false);
  const [busyName, setBusyName] = useState<string | null>(null);

  const profile = companion.profile;
  const config = profile?.skills ?? EMPTY_SKILL_CONFIG;
  // No profile yet (or its fetch failed) means no grant set to read: showing the
  // auto-injected defaults as "granted" would render opt-outs the user has
  // actually made, and every grant control would silently no-op.
  const profileReady = profile != null;
  const localeKey = i18n.language;
  const display = useCallback(
    (skill: CatalogSkillInfo) => resolveSkillDisplay(skill, localeKey),
    [localeKey]
  );

  const entries = useMemo(
    () =>
      buildSkillEntries({
        generated: data.generated,
        catalog: profileReady ? data.catalog : [],
        autoNames: profileReady ? data.autoNames : NO_AUTO_NAMES,
        config,
        missingDescription: t('nomi.skills.configMissingHint', {
          defaultValue: '这个 Skill 当前未安装；重新安装后会自动恢复。',
        }),
        display,
      }),
    [config, data.autoNames, data.catalog, data.generated, display, profileReady, t]
  );

  const visibleEntries = useMemo(() => filterSkillEntries(entries, filter), [entries, filter]);
  const draftCount = useMemo(() => countDrafts(entries), [entries]);

  useEffect(() => {
    onAttentionChange?.(draftCount > 0);
  }, [draftCount, onAttentionChange]);

  // A companion switch invalidates every selection-shaped bit of local state.
  useEffect(() => {
    setSelectedKey(null);
    setEditing(false);
    setFilter('all');
  }, [companionId]);

  const selected = entries.find((entry) => entry.key === selectedKey) ?? null;

  const openEntry = useCallback(
    (entry: SkillEntry, edit: boolean) => {
      setSelectedKey(entry.key);
      // Re-clicking the row that is already open must not drop edit mode (and
      // with it whatever the user has typed into SKILL.md).
      if (edit) setEditing(true);
      else if (entry.key !== selectedKey) setEditing(false);
    },
    [selectedKey]
  );

  /** Grant/revoke a catalog capability through the shared reducer. */
  const setGrant = useCallback(
    async (name: string, granted: boolean): Promise<boolean> => {
      if (!profile) return false;
      const skills = toggleCompanionSkill(profile.skills, data.autoNames, name, granted);
      setBusyName(name);
      try {
        await companion.patchCompanion({ skills });
        return true;
      } catch (error) {
        Message.error(String(error));
        return false;
      } finally {
        setBusyName(null);
      }
    },
    [companion, data.autoNames, profile]
  );

  const revokeGrant = useCallback(
    async (name: string) => {
      if (await setGrant(name, false)) {
        Message.success(t('nomi.skills.revoked', { defaultValue: '已取消授予' }));
      }
    },
    [setGrant, t]
  );

  const decide = useCallback(
    (entry: SkillEntry, accept: boolean) => {
      if (entry.kind !== 'generated') return;
      const skillId = entry.skill.companion_skill_id;
      if (accept) {
        void data.decide(skillId, true).then((ok) => {
          if (ok) Message.success(t('nomi.skills.acceptedOk', { defaultValue: '已采纳，技能开始生效' }));
        });
        return;
      }
      Modal.confirm({
        title: t('nomi.skills.rejectConfirm', { defaultValue: '拒绝这个技能？' }),
        content: t('nomi.skills.rejectConfirmBody', {
          defaultValue: '拒绝后它会被归档，不会在对话里使用。',
        }),
        okButtonProps: { status: 'danger' },
        onOk: () => data.decide(skillId, false),
      });
    },
    [data, t]
  );

  const asideSubtitle = (entry: SkillEntry): string =>
    entry.kind === 'generated'
      ? t('nomi.skills.sourceGenerated', { defaultValue: '自动生成' })
      : `${t('nomi.skills.sourceCatalog', { defaultValue: '已配置' })} · ${
          entry.isAuto
            ? t('nomi.skills.configDefault', { defaultValue: '默认能力' })
            : t('nomi.skills.configOptional', { defaultValue: '额外能力' })
        }`;

  const aside = useAsidePortal(
    selected ? (
      <ContentAside
        title={selected.name}
        subtitle={asideSubtitle(selected)}
        onClose={() => setSelectedKey(null)}
        storageKey='nomifun:nomi-aside-skills'
      >
        {selected.kind === 'generated' ? (
          <GeneratedSkillDetail
            companionId={companionId}
            entry={selected}
            editing={editing}
            onEditingChange={setEditing}
            onDecide={(accept) => decide(selected, accept)}
            onSaved={() => void data.refresh()}
          />
        ) : (
          <CatalogSkillDetail
            entry={selected}
            busy={busyName === selected.name}
            disabled={busyName !== null || !profileReady}
            onRevoke={() => void revokeGrant(selected.name)}
          />
        )}
      </ContentAside>
    ) : null
  );

  const body = (
    <div className='flex flex-col gap-16px py-8px'>
      <NomiSettingSection
        title={t('nomi.skills.sectionTitle', { defaultValue: '技能' })}
        description={t('nomi.skills.sectionDesc', {
          defaultValue: '这个伙伴会的能力：你从技能库授予的，和它在真实工作里自己沉淀的。',
        })}
      >
        <div className='flex flex-col gap-12px'>
          <SkillsToolbar
            filter={filter}
            onFilterChange={setFilter}
            hasDrafts={draftCount > 0}
            grantsDisabled={!profileReady}
            onLearnFromSession={() => setLearnOpen(true)}
            onAddCapability={() => setPickerOpen(true)}
          />
          {data.initialLoading || (!profileReady && companion.loading) ? (
            <div className='flex justify-center py-40px'>
              <Spin />
            </div>
          ) : entries.length === 0 ? (
            <SkillsEmptyState
              addDisabled={!profileReady}
              onAddCapability={() => setPickerOpen(true)}
            />
          ) : visibleEntries.length === 0 ? (
            <div className='py-40px text-center text-13px text-t-tertiary'>
              {t('nomi.skills.filterEmpty', { defaultValue: '这个筛选下还没有技能' })}
            </div>
          ) : (
            <div
              className='transition-opacity duration-150'
              style={{ opacity: data.loading ? 0.6 : 1 }}
            >
              <NomiSettingList>
                {visibleEntries.map((entry) => (
                  <SkillListRow
                    key={entry.key}
                    entry={entry}
                    selected={entry.key === selectedKey}
                    busy={busyName === entry.name}
                    grantDisabled={busyName !== null || !profileReady}
                    onSelect={() => openEntry(entry, false)}
                    onEdit={() => openEntry(entry, true)}
                    onDecide={(accept) => decide(entry, accept)}
                    onRevoke={() => void revokeGrant(entry.name)}
                  />
                ))}
              </NomiSettingList>
            </div>
          )}
        </div>
      </NomiSettingSection>

      <CatalogPickerModal
        visible={pickerOpen}
        onClose={() => setPickerOpen(false)}
        catalog={data.catalog}
        autoNames={data.autoNames}
        config={config}
        localeKey={localeKey}
        busyName={busyName}
        disabled={busyName !== null || !profileReady}
        onToggle={(name, granted) => void setGrant(name, granted)}
      />
      <LearnFromSessionModal
        visible={learnOpen}
        onClose={() => setLearnOpen(false)}
        onSubmit={data.learnFromSession}
      />
    </div>
  );

  return (
    <>
      {body}
      {aside}
    </>
  );
};

export default SkillsTab;
