/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { Button, Input, Message, Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { emitter } from '@/renderer/utils/emitter';
import {
  WorkspaceDirectoryUnavailableError,
  WorkspaceFolderSelect,
  validateExistingWorkspaceDirectory,
} from '@/renderer/components/workspace';
import {
  buildLaunchCommand,
  formatCommandPreview,
  getPreset,
  parseCommandPreview,
  TERMINAL_PRESETS,
  type TerminalPresetId,
} from './launchPresets';
import ExtendedCapabilitiesPanel from './ExtendedCapabilitiesPanel';
import LabelWithTip from './LabelWithTip';
import { addRecentLaunchCommand, getRecentLaunchCommands } from './recentLaunchCommands';

const TerminalCreatePage: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useTranslation();
  const [presetId, setPresetId] = useState<TerminalPresetId>('shell');
  const [cwd, setCwd] = useState('');
  const [commandPreview, setCommandPreview] = useState(() =>
    formatCommandPreview(buildLaunchCommand('shell'))
  );
  const commandPreset = useRef<TerminalPresetId>('shell');
  const [creating, setCreating] = useState(false);
  const launchOwner = useRef<{ busy: boolean } | null>(null);
  // Recent custom launch commands (read once on mount; the page unmounts on launch).
  const [recentCommands] = useState<string[]>(() => getRecentLaunchCommands());
  // Optional knowledge bases bound at creation (mounted into {cwd}/.nomi/knowledge/).
  const [knowledgeBases, setKnowledgeBases] = useState<IKnowledgeBase[]>([]);
  const [kbIds, setKbIds] = useState<string[]>([]);

  // Preset working directory passed via navigation state (sidebar workpath
  // drawer → "new terminal session"). Each navigation owns its launch, even
  // when the same route stays mounted; ordinary edits never reset the draft.
  useLayoutEffect(() => {
    launchOwner.current = { busy: false };
    setCreating(false);
    const presetCwd = (location.state as { cwd?: string } | null)?.cwd;
    setCwd(typeof presetCwd === 'string' ? presetCwd : '');
    return () => { launchOwner.current = null; };
  }, [location.key, location.state]);

  useEffect(() => {
    let cancelled = false;
    void ipcBridge.knowledge.listBases
      .invoke()
      .then((list) => {
        if (!cancelled) setKnowledgeBases(list);
      })
      .catch(() => {
        /* knowledge platform unavailable → hide the picker */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const preset = useMemo(() => getPreset(presetId), [presetId]);

  // Keep the editable command preview in sync with the selected preset.
  useEffect(() => {
    if (commandPreset.current === presetId) return;
    commandPreset.current = presetId;
    setCommandPreview(formatCommandPreview(buildLaunchCommand(presetId)));
  }, [presetId]);

  const handleLaunch = async () => {
    const owner = launchOwner.current;
    if (!owner || owner.busy) return;
    const isCurrent = () => launchOwner.current === owner;
    const { command, args } = parseCommandPreview(commandPreview);
    if (!command) {
      Message.warning(t('terminal.create.commandRequired'));
      return;
    }
    owner.busy = true;
    setCreating(true);
    try {
      const launchCwd = cwd.trim()
        ? await validateExistingWorkspaceDirectory(cwd)
        : '';
      if (!isCurrent()) return;
      if (launchCwd !== cwd) setCwd(launchCwd);
      const session = await ipcBridge.terminal.create.invoke({
        cwd: launchCwd,
        command,
        args,
        backend: preset.backend,
        mode: preset.backend ? 'full-auto' : undefined,
        // Defer the PTY spawn until XtermView mounts and sends the first resize
        // with the real fitted size, so a full-screen TUI (claude) draws at the
        // correct dimensions from frame one — no garble-until-you-resize.
        defer_spawn: true,
        knowledge_base_ids: kbIds.length > 0 ? kbIds : undefined,
      });
      if (!isCurrent()) return;
      // Remember the launched command for quick reuse — only for the custom preset.
      if (presetId === 'shell') addRecentLaunchCommand(commandPreview);
      emitter.emit('terminal.list.refresh');
      navigate(`/terminal/${session.terminal_id}`);
    } catch (err) {
      if (isCurrent()) {
        Message.error(
          err instanceof WorkspaceDirectoryUnavailableError
            ? t('terminal.create.workspaceUnavailable', { workspacePath: err.workspacePath })
            : err instanceof Error ? err.message : String(err)
        );
      }
    } finally {
      if (isCurrent()) {
        owner.busy = false;
        setCreating(false);
      }
    }
  };

  return (
    <div className='flex h-full min-h-0 items-start justify-center overflow-y-auto bg-fill-1 p-24px'>
      <div className='w-[min(640px,100%)] rounded-16px bg-fill-0 p-24px shadow-sm'>
        <h2 className='mb-4px text-18px font-semibold text-t-primary'>{t('terminal.create.title')}</h2>
        <p className='mb-20px text-13px text-t-secondary'>{t('terminal.create.subtitle')}</p>

        {/* Workspace path → cd */}
        <label className='mb-6px block text-14px font-medium text-t-primary'>{t('terminal.create.workspace')}</label>
        <div className='mb-16px'>
          <WorkspaceFolderSelect
            value={cwd}
            onChange={setCwd}
            onClear={() => setCwd('')}
            placeholder={t('terminal.create.workspacePlaceholder')}
            recentLabel={t('terminal.create.recent')}
            chooseDifferentLabel={t('terminal.create.chooseFolder')}
          />
        </div>

        {/* Preset */}
        <LabelWithTip label={t('terminal.create.preset')} tip={t('terminal.create.presetHint')} />
        <Select className='mb-16px' value={presetId} onChange={(v) => setPresetId(v as TerminalPresetId)}>
          {TERMINAL_PRESETS.map((p) => (
            <Select.Option key={p.id} value={p.id}>
              {t(p.labelKey)}
            </Select.Option>
          ))}
        </Select>

        {/* Editable launch command preview */}
        <LabelWithTip label={t('terminal.create.command')} tip={t('terminal.create.commandHint')} />
        <Input className={`font-mono ${presetId === 'shell' && recentCommands.length > 0 ? 'mb-8px' : 'mb-20px'}`} value={commandPreview} onChange={setCommandPreview} onInput={event => setCommandPreview((event.target as HTMLInputElement).value)} placeholder='$SHELL' />

        {/* Recent launch commands — custom preset only; click to fill the command field */}
        {presetId === 'shell' && recentCommands.length > 0 && (
          <div className='mb-20px'>
            <div className='mb-6px text-12px text-t-tertiary'>{t('terminal.create.recentCommands')}</div>
            <div className='flex flex-col gap-2px'>
              {recentCommands.map((cmd) => (
                <button
                  key={cmd}
                  type='button'
                  title={cmd}
                  onClick={() => setCommandPreview(cmd)}
                  className='block w-full cursor-pointer truncate rounded-6px b-none bg-fill-1 px-8px py-4px text-left font-mono text-12px text-t-secondary appearance-none hover:bg-fill-2'
                >
                  {cmd}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className='flex justify-end gap-8px'>
          <Button onClick={() => navigate(-1)}>{t('common.cancel')}</Button>
          <Button type='primary' loading={creating} onClick={handleLaunch}>
            {t('terminal.create.launch')}
          </Button>
        </div>

        {/* Optional Knowledge mount and external CLI registration. */}
        <ExtendedCapabilitiesPanel
          cwd={cwd}
          command={commandPreview}
          knowledgeBases={knowledgeBases}
          kbIds={kbIds}
          onKbIdsChange={setKbIds}
        />
      </div>
    </div>
  );
};

export default TerminalCreatePage;
