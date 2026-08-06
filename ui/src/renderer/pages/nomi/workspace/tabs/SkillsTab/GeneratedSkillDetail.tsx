/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message, Spin } from '@arco-design/web-react';
import { Check, Close, Edit } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { CompanionId } from '@/common/types/ids';
import SkillButton from './SkillButton';
import type { GeneratedSkillEntry } from './unify';

interface GeneratedSkillDetailProps {
  companionId: CompanionId;
  entry: GeneratedSkillEntry;
  /** Edit mode is owned by the tab, so the row's 编辑 works on every click. */
  editing: boolean;
  onEditingChange: (editing: boolean) => void;
  onDecide: (accept: boolean) => void;
  onSaved: () => void;
}

const MetaRow: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className='flex flex-col gap-2px'>
    <span className='text-11px leading-16px text-t-tertiary'>{label}</span>
    <span className='text-13px leading-18px text-t-primary break-words'>{children}</span>
  </div>
);

/**
 * The generated skill's detail surface: what it is, and its SKILL.md — read-only
 * until the user asks to edit. Living in the aside (not a modal) means the file
 * stays open while the user keeps scanning the list.
 */
const GeneratedSkillDetail: React.FC<GeneratedSkillDetailProps> = ({
  companionId,
  entry,
  editing,
  onEditingChange,
  onDecide,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [content, setContent] = useState('');
  const [draft, setDraft] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const mounted = useRef(true);
  const skillId = entry.skill.companion_skill_id;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    ipcBridge.companion.getSkillContent
      .invoke({ companion_id: companionId, companion_skill_id: skillId })
      .then((result) => {
        if (cancelled) return;
        setContent(result.content);
        setDraft(result.content);
      })
      .catch((error) => {
        if (cancelled) return;
        Message.error(String(error));
        setContent('');
        setDraft('');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [companionId, skillId]);

  // Edit mode is a parent prop, so entering/leaving it never re-runs the fetch
  // above and never throws away what the user has typed into SKILL.md.
  const save = useCallback(async () => {
    setSaving(true);
    try {
      await ipcBridge.companion.writeSkillContent.invoke({
        companion_id: companionId,
        companion_skill_id: skillId,
        content: draft,
      });
      if (!mounted.current) return;
      setContent(draft);
      onEditingChange(false);
      Message.success(t('nomi.skills.saveOk', { defaultValue: '已保存' }));
      onSaved();
    } catch (error) {
      // Backend BadRequest (missing frontmatter / empty description) lands here.
      Message.error(String(error));
    } finally {
      if (mounted.current) setSaving(false);
    }
  }, [companionId, draft, onEditingChange, onSaved, skillId, t]);

  return (
    <div className='flex flex-col gap-16px'>
      {entry.status === 'draft' && (
        <div className='flex flex-col gap-8px rd-12px border border-solid border-[var(--color-border-2)] p-12px'>
          <span className='text-12px leading-18px text-t-secondary'>
            {t('nomi.skills.draftReviewHint', {
              defaultValue: '这是伙伴刚起草的技能，采纳后才会在对话里生效。',
            })}
          </span>
          <div className='flex items-center gap-8px'>
            <SkillButton
              tone='primary'
              icon={<Check theme='outline' size='12' fill='currentColor' strokeWidth={4} />}
              onClick={() => onDecide(true)}
            >
              {t('nomi.skills.accept', { defaultValue: '采纳' })}
            </SkillButton>
            <SkillButton
              tone='danger'
              icon={<Close theme='outline' size='12' fill='currentColor' strokeWidth={4} />}
              onClick={() => onDecide(false)}
            >
              {t('nomi.skills.reject', { defaultValue: '拒绝' })}
            </SkillButton>
          </div>
        </div>
      )}

      <MetaRow label={t('nomi.skills.metaOrigin', { defaultValue: '生成来源' })}>
        {t(`nomi.skills.source_${entry.skill.source}`, { defaultValue: entry.skill.source })}
      </MetaRow>
      <MetaRow label={t('nomi.skills.metaUpdated', { defaultValue: '最近更新' })}>
        {new Date(entry.skill.updated_at).toLocaleString()}
      </MetaRow>

      <div className='flex flex-col gap-8px'>
        <div className='flex items-center justify-between gap-8px'>
          <span className='text-13px font-600 text-t-primary'>SKILL.md</span>
          {editing ? (
            <div className='flex items-center gap-6px'>
              <SkillButton
                onClick={() => {
                  onEditingChange(false);
                  setDraft(content);
                }}
              >
                {t('nomi.skills.cancel', { defaultValue: '取消' })}
              </SkillButton>
              <SkillButton tone='primary' disabled={saving} onClick={() => void save()}>
                {t('nomi.skills.save', { defaultValue: '保存' })}
              </SkillButton>
            </div>
          ) : (
            <SkillButton
              icon={<Edit theme='outline' size='12' fill='currentColor' strokeWidth={3} />}
              onClick={() => onEditingChange(true)}
            >
              {t('nomi.skills.edit', { defaultValue: '编辑' })}
            </SkillButton>
          )}
        </div>
        {loading ? (
          <div className='flex justify-center py-32px'>
            <Spin />
          </div>
        ) : (
          <Input.TextArea
            value={editing ? draft : content}
            onChange={setDraft}
            readOnly={!editing}
            autoSize={{ minRows: 14, maxRows: 40 }}
            className='font-mono text-12px leading-18px'
          />
        )}
      </div>
    </div>
  );
};

export { MetaRow };
export default GeneratedSkillDetail;
