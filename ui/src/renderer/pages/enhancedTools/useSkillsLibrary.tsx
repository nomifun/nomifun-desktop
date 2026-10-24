/** Skill library state shared by the enhanced-tools header and library panel. */
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { SkillInfo } from '@/common/types/skill';
import { resolveLocaleKey } from '@/common/utils';
import {
  readInstalledMarketState,
  reconcileInstalledMarketState,
  writeInstalledMarketState,
} from '@/renderer/services/skills/skillMarketProvenance';
import { useSkillDisplay } from '@/renderer/services/skills/useSkillDisplay';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ExternalAgentSkillSource } from '../settings/skill/agentSkillImportUtils';

export const useSkillsLibrary = () => {
  const { t, i18n } = useTranslation();
  const localeKey = resolveLocaleKey(i18n.language);
  const getSkillDisplay = useSkillDisplay(localeKey);
  const [message, messageContext] = useArcoMessage({ maxCount: 10 });
  const fetchRequestRef = useRef(0);
  const mountedRef = useRef(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [availableSkills, setAvailableSkills] = useState<SkillInfo[]>([]);
  const [skillPaths, setSkillPaths] = useState<{ user_skills_dir: string } | null>(null);
  const [autoInjectedNames, setAutoInjectedNames] = useState<Set<string>>(() => new Set());
  const [searchQuery, setSearchQuery] = useState('');
  const [agentImportVisible, setAgentImportVisible] = useState(false);
  const [detailSkillName, setDetailSkillName] = useState<string | null>(null);
  const [pendingDeleteName, setPendingDeleteName] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  const refresh = useCallback(async (): Promise<boolean> => {
    if (!mountedRef.current) return false;
    const requestId = ++fetchRequestRef.current;
    setLoading(true);
    setError(null);
    try {
      const [skillsResult, pathsResult, autoSkillsResult] = await Promise.allSettled([
        ipcBridge.fs.listAvailableSkills.invoke(),
        ipcBridge.fs.getSkillPaths.invoke(),
        ipcBridge.fs.listBuiltinAutoSkills.invoke(),
      ]);
      if (requestId !== fetchRequestRef.current) return false;
      if (skillsResult.status === 'rejected') throw skillsResult.reason;
      const skills = skillsResult.value as SkillInfo[];
      writeInstalledMarketState(
        reconcileInstalledMarketState(readInstalledMarketState(), skills.map((skill) => skill.name))
      );
      setAvailableSkills(skills);
      if (pathsResult.status === 'fulfilled') setSkillPaths({ user_skills_dir: pathsResult.value.user_skills_dir });
      else console.error('Failed to fetch skill paths:', pathsResult.reason);
      if (autoSkillsResult.status === 'fulfilled') setAutoInjectedNames(new Set(autoSkillsResult.value.map((skill) => skill.name)));
      else console.error('Failed to fetch built-in auto skills:', autoSkillsResult.reason);
      return true;
    } catch (cause) {
      if (requestId !== fetchRequestRef.current) return false;
      console.error('Failed to fetch skills:', cause);
      setError(t('settings.skillsHub.fetchError', { defaultValue: 'Failed to fetch skills' }));
      return false;
    } finally {
      if (requestId === fetchRequestRef.current) setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    mountedRef.current = true;
    void refresh();
    return () => {
      mountedRef.current = false;
      // An older request must not reconcile provenance after a market install.
      fetchRequestRef.current += 1;
    };
  }, [refresh]);

  const filteredSkills = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) return availableSkills;
    return availableSkills.filter((skill) => {
      const display = getSkillDisplay(skill);
      return `${skill.name} ${skill.description} ${display.name} ${display.description}`.toLowerCase().includes(query);
    });
  }, [availableSkills, getSkillDisplay, searchQuery]);
  const detailSkill = useMemo(
    () => availableSkills.find((skill) => skill.name === detailSkillName) ?? null,
    [availableSkills, detailSkillName]
  );
  const pendingDeleteSkill = useMemo(
    () => availableSkills.find((skill) => skill.name === pendingDeleteName && skill.source === 'custom') ?? null,
    [availableSkills, pendingDeleteName]
  );

  const importSkill = async (skillPath: string) => {
    try {
      const result = await ipcBridge.fs.importSkillWithSymlink.invoke({ skill_path: skillPath });
      if (!mountedRef.current) return;
      const importedNames = result.skill_names?.length ? result.skill_names : result.skill_name ? [result.skill_name] : [];
      const count = importedNames.length;
      const names = importedNames.join(', ');
      message.success(t('settings.skillsHub.importSuccessDetailed', {
        count,
        names,
        defaultValue: count > 1 ? `Imported ${count} skills: ${names}` : `Imported skill: ${names}`,
      }));
      setSearchQuery('');
      await refresh();
    } catch (cause) {
      if (!mountedRef.current) return;
      console.error('Failed to import skill:', cause);
      const detail = isBackendHttpError(cause) ? cause.backendMessage : '';
      message.error(detail
        ? t('settings.skillsHub.importErrorDetailed', { detail, defaultValue: `Error importing skill: ${detail}` })
        : t('settings.skillsHub.importError', { defaultValue: 'Error importing skill' }));
    }
  };

  // Tauri cannot offer files and folders in one picker. Keep both import paths.
  const importFolder = async () => {
    try {
      const result = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
      if (mountedRef.current && result?.length) await importSkill(result[0]);
    } catch (cause) {
      console.error('Failed to open folder dialog:', cause);
    }
  };
  const importZip = async () => {
    try {
      const result = await ipcBridge.dialog.showOpen.invoke({
        properties: ['openFile'],
        filters: [{ name: 'Skill zip archives', extensions: ['zip'] }],
      });
      if (mountedRef.current && result?.length) await importSkill(result[0]);
    } catch (cause) {
      console.error('Failed to open zip dialog:', cause);
    }
  };

  const loadAgentSkillSources = useCallback(async (): Promise<ExternalAgentSkillSource[]> =>
    (await ipcBridge.fs.detectAndCountExternalSkills.invoke()) as ExternalAgentSkillSource[], []);
  const onAgentSkillsImported = useCallback(async () => {
    if (!mountedRef.current) return;
    setSearchQuery('');
    await refresh();
  }, [refresh]);

  const deletePendingSkill = async () => {
    if (!pendingDeleteSkill || deleting) return;
    setDeleting(true);
    try {
      await ipcBridge.fs.deleteSkill.invoke({ skill_name: pendingDeleteSkill.name });
      if (!mountedRef.current) return;
      setPendingDeleteName(null);
      message.success(t('settings.skillsHub.deleteSuccess', { defaultValue: 'Skill deleted' }));
      await refresh();
    } catch (cause) {
      if (!mountedRef.current) return;
      console.error('Failed to delete skill:', cause);
      const detail = isBackendHttpError(cause) ? cause.backendMessage : '';
      message.error(detail
        ? t('settings.skillsHub.deleteErrorDetailed', { detail, defaultValue: `Error deleting skill: ${detail}` })
        : t('settings.skillsHub.deleteError', { defaultValue: 'Error deleting skill' }));
    } finally {
      if (mountedRef.current) setDeleting(false);
    }
  };
  const confirmDelete = (skill: SkillInfo) => {
    if (skill.source === 'custom' && !deleting) setPendingDeleteName(skill.name);
  };

  return {
    localeKey, loading, error, availableSkills, filteredSkills, skillPaths, autoInjectedNames,
    searchQuery, setSearchQuery, refresh, importFolder, importZip,
    importFromAgent: () => setAgentImportVisible(true),
    openDetails: (skill: SkillInfo) => setDetailSkillName(skill.name),
    closeDetails: () => setDetailSkillName(null),
    confirmDelete, pendingDeleteSkill, deleting, deletePendingSkill,
    cancelDelete: () => { if (!deleting) setPendingDeleteName(null); },
    detailSkill, agentImportVisible,
    closeAgentImport: () => setAgentImportVisible(false),
    loadAgentSkillSources, onAgentSkillsImported, messageContext,
  };
};

export type SkillsLibraryController = ReturnType<typeof useSkillsLibrary>;
