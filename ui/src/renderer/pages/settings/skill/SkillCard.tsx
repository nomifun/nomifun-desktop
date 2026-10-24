/** Compact library item. Skills are inspected here and selected in a Session. */
import type { SkillInfo } from '@/common/types/skill';
import { useSkillDisplay } from '@/renderer/services/skills/useSkillDisplay';
import { Button } from '@arco-design/web-react';
import { Delete, FileText, Lightning } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { normalizeTestId } from './skillPresentation';

type SkillCardProps = {
  skill: SkillInfo;
  localeKey: string;
  isAutoInjected: boolean;
  onOpenDetails: (skill: SkillInfo) => void;
  onDelete: (skill: SkillInfo) => void;
  highlighted?: boolean;
  cardRef?: (element: HTMLDivElement | null) => void;
};

const SkillCard: React.FC<SkillCardProps> = ({
  skill, localeKey, isAutoInjected, onOpenDetails, onDelete, highlighted = false, cardRef,
}) => {
  const { t } = useTranslation();
  const display = useSkillDisplay(localeKey)(skill);
  const testId = normalizeTestId(skill.name);
  const source = isAutoInjected ? 'auto' : skill.source;
  const sourceLabel = isAutoInjected
    ? t('settings.skillsHub.sourceAuto', { defaultValue: 'Auto' })
    : skill.source === 'custom'
      ? t('settings.skillsHub.custom', { defaultValue: 'Custom' })
      : t('settings.skillsHub.builtin', { defaultValue: 'Built-in' });
  const detailsLabel = t('settings.skillsHub.detailTitle', { defaultValue: 'Skill Details' });

  return (
    <div
      ref={cardRef}
      data-testid={`skill-card-${testId}`}
      className={`enhanced-tools-skill-card${highlighted ? ' enhanced-tools-skill-card--highlighted' : ''}`}
    >
      <div
        role='button'
        tabIndex={0}
        aria-label={`${detailsLabel}: ${display.name}`}
        className='enhanced-tools-skill-main'
        onClick={() => onOpenDetails(skill)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onOpenDetails(skill);
          }
        }}
      >
        <div className='enhanced-tools-skill-icon' aria-hidden='true'>
          {isAutoInjected ? <Lightning size={18} /> : <FileText size={18} />}
        </div>
        <div className='enhanced-tools-skill-copy'>
          <span className='enhanced-tools-skill-name' title={display.name}>{display.name}</span>
          <span className='enhanced-tools-skill-description' title={display.description || undefined}>
            {display.description || t('settings.skillsHub.noDescription', { defaultValue: 'No description provided.' })}
          </span>
        </div>
        <div className='enhanced-tools-skill-meta'>
          <span className={`enhanced-tools-skill-source enhanced-tools-skill-source--${source}`}>{sourceLabel}</span>
          <span className='enhanced-tools-skill-details'>{detailsLabel}</span>
        </div>
      </div>
      {skill.source === 'custom' && (
        <Button
          type='text'
          size='small'
          className='enhanced-tools-skill-delete'
          data-testid={`btn-delete-${testId}`}
          aria-label={`${t('common.delete', { defaultValue: 'Delete' })} ${display.name}`}
          title={t('common.delete', { defaultValue: 'Delete' })}
          icon={<Delete size={15} />}
          onClick={() => onDelete(skill)}
        />
      )}
    </div>
  );
};

export default SkillCard;
