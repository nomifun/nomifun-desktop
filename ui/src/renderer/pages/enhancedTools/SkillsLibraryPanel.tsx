/** The complete installed library; the enhanced-tools page owns its toolbar. */
import { Button, Modal } from '@arco-design/web-react';
import { FolderOpen, Info, Refresh } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import AgentSkillImportDrawer from '../settings/skill/AgentSkillImportDrawer';
import SkillCard from '../settings/skill/SkillCard';
import SkillDetailDrawer from '../settings/skill/SkillDetailDrawer';
import type { SkillsLibraryController } from './useSkillsLibrary';
import { useSkillDisplay } from '@/renderer/services/skills/useSkillDisplay';

type SkillsLibraryPanelProps = { library: SkillsLibraryController; visible?: boolean };

const SkillsLibraryPanel: React.FC<SkillsLibraryPanelProps> = ({ library, visible = true }) => {
  const { t } = useTranslation();
  const getSkillDisplay = useSkillDisplay(library.localeKey);
  const [searchParams, setSearchParams] = useSearchParams();
  const highlightName = searchParams.get('highlight');
  const [highlightedSkill, setHighlightedSkill] = useState<string | null>(null);
  const skillRefs = useRef<Record<string, HTMLDivElement | null>>({});

  useEffect(() => {
    if (visible && highlightName) library.setSearchQuery('');
  }, [visible, highlightName, library.setSearchQuery]);

  useEffect(() => {
    if (!visible || !highlightName || library.loading || library.error) return;
    const element = skillRefs.current[highlightName];
    if (!element) return;
    const frame = requestAnimationFrame(() => {
      element.scrollIntoView({ behavior: 'smooth', block: 'center' });
      setHighlightedSkill(highlightName);
      const next = new URLSearchParams(searchParams);
      next.delete('highlight');
      setSearchParams(next, { replace: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, highlightName, library.loading, library.error, library.filteredSkills, searchParams, setSearchParams]);

  useEffect(() => {
    if (!highlightedSkill) return;
    const timer = setTimeout(() => setHighlightedSkill(null), 2000);
    return () => clearTimeout(timer);
  }, [highlightedSkill]);

  return (
    <div className='enhanced-tools-skill-library' data-testid='skills-library-surface' aria-busy={library.loading}>
      {library.messageContext}
      {library.error ? (
        <div className='enhanced-tools-empty' role='alert' data-testid='skills-library-error'>
          <p>{library.error}</p>
          <Button icon={<Refresh size={14} />} onClick={() => void library.refresh()}>
            {t('common.retry', { defaultValue: 'Retry' })}
          </Button>
        </div>
      ) : library.filteredSkills.length > 0 ? (
        <div className='enhanced-tools-skill-grid' data-testid='skills-library-grid'>
          {library.filteredSkills.map((skill) => (
            <SkillCard
              key={skill.name}
              skill={skill}
              localeKey={library.localeKey}
              isAutoInjected={library.autoInjectedNames.has(skill.name)}
              onOpenDetails={library.openDetails}
              onDelete={library.confirmDelete}
              highlighted={highlightedSkill === skill.name}
              cardRef={(element) => { skillRefs.current[skill.name] = element; }}
            />
          ))}
        </div>
      ) : (
        <div className='enhanced-tools-empty' data-testid='skills-library-empty'>
          {library.loading
            ? t('common.loading', { defaultValue: 'Please wait...' })
            : library.availableSkills.length === 0
              ? t('settings.skillsHub.noSkills', { defaultValue: 'No skills found. Import some to get started.' })
              : t('settings.skillsHub.noMatch', { defaultValue: 'No skills match the current filters.' })}
        </div>
      )}

      <div className='enhanced-tools-skill-footer'>
        {library.skillPaths && (
          <div className='enhanced-tools-skill-path'>
            <FolderOpen size={14} className='shrink-0' />
            <span title={library.skillPaths.user_skills_dir} data-testid='skill-paths-display'>
              {library.skillPaths.user_skills_dir}
            </span>
          </div>
        )}
        <div className='enhanced-tools-skill-tip'>
          <Info size={15} className='shrink-0' />
          <span>{t('settings.skillsHub.tipContent')}</span>
        </div>
      </div>

      <SkillDetailDrawer
        visible={visible && library.detailSkill !== null}
        skill={library.detailSkill}
        localeKey={library.localeKey}
        isAutoInjected={library.detailSkill !== null && library.autoInjectedNames.has(library.detailSkill.name)}
        onClose={library.closeDetails}
      />
      <Modal
        title={t('settings.skillsHub.deleteConfirmTitle', { defaultValue: 'Delete Skill' })}
        visible={visible && library.pendingDeleteSkill !== null}
        onCancel={library.cancelDelete}
        onOk={library.deletePendingSkill}
        confirmLoading={library.deleting}
        cancelButtonProps={{ disabled: library.deleting }}
        okButtonProps={{ status: 'danger' }}
        okText={t('common.delete', { defaultValue: 'Delete' })}
        cancelText={t('common.cancel', { defaultValue: 'Cancel' })}
        wrapClassName='modal-delete-skill'
      >
        {library.pendingDeleteSkill && t('settings.skillsHub.deleteConfirmContent', {
          name: getSkillDisplay(library.pendingDeleteSkill).name,
          defaultValue: `Are you sure you want to delete "${getSkillDisplay(library.pendingDeleteSkill).name}"?`,
        })}
      </Modal>
      <AgentSkillImportDrawer
        visible={visible && library.agentImportVisible}
        onClose={library.closeAgentImport}
        existingSkillNames={library.availableSkills.map((skill) => skill.name)}
        onImported={library.onAgentSkillsImported}
        loadSources={library.loadAgentSkillSources}
      />
    </div>
  );
};

export default SkillsLibraryPanel;
