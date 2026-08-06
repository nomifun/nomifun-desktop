/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Popover } from '@arco-design/web-react';
import { Plus, Server } from '@icon-park/react';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { IApiSshHost } from '@/common/adapter/ipcBridge';
import { useOpenSshSession } from '@renderer/pages/conversation/hooks/useOpenSshSession';
import { SshHostFormModal } from '@renderer/pages/settings/SshHostSettings/SshHostManagement';

export interface RemoteSessionPopoverProps {
  /** Class string for the trigger, so it matches its siblings in the action grid. */
  buttonClassName: string;
  /** A session was opened — the sidebar clears batch mode / closes the overlay. */
  onLaunched?: () => void;
}

/**
 * RemoteSessionPopover — the sidebar's remote-session entry.
 *
 * Reaching a remote session used to mean Settings → remote hosts → add → back to
 * the home page. Here the saved host book *is* the menu: one click on a row
 * starts a session on that host, and an empty book leads with the same add-host
 * form the settings screen uses, which starts the session as soon as it saves.
 *
 * The host list is fetched only after the popover has been opened once (the
 * sidebar is mounted on every session route, and an unopened menu should not
 * cost a request). The SWR key is shared with the settings host book, so the two
 * screens never disagree about what is saved.
 */
const RemoteSessionPopover: React.FC<RemoteSessionPopoverProps> = ({ buttonClassName, onLaunched }) => {
  const { t } = useTranslation();
  const openSshSession = useOpenSshSession();
  const [visible, setVisible] = useState(false);
  const [everOpened, setEverOpened] = useState(false);
  const [formVisible, setFormVisible] = useState(false);
  const [startingId, setStartingId] = useState<IApiSshHost['sshHostId'] | null>(null);

  const { data: hosts, isLoading, mutate } = useSWR(everOpened ? 'ssh-hosts.list' : null, () =>
    ipcBridge.ssh.list.invoke()
  );

  const handleVisibleChange = useCallback((next: boolean) => {
    setVisible(next);
    if (next) setEverOpened(true);
  }, []);

  const launch = useCallback(
    async (host: IApiSshHost) => {
      if (startingId) return;
      setStartingId(host.sshHostId);
      const opened = await openSshSession(host);
      setStartingId(null);
      // A failed launch keeps the menu open so its error toast has a context.
      if (!opened) return;
      setVisible(false);
      onLaunched?.();
    },
    [onLaunched, openSshSession, startingId]
  );

  const openForm = useCallback(() => {
    setVisible(false);
    setFormVisible(true);
  }, []);

  const rows = hosts ?? [];
  const busy = startingId !== null;

  const content = (
    <div className='w-244px p-6px flex flex-col gap-2px' data-testid='remote-session-menu'>
      {rows.length > 0 && (
        <div className='px-6px pt-2px pb-4px text-11px leading-16px text-t-tertiary'>
          {t('sessionList.remoteHint')}
        </div>
      )}

      {isLoading && rows.length === 0 ? (
        <div className='px-6px py-10px text-12px text-t-tertiary'>{t('sessionList.remoteLoading')}</div>
      ) : rows.length === 0 ? (
        <div className='px-6px py-8px flex flex-col gap-6px'>
          <span className='text-12px leading-18px text-t-secondary'>{t('ssh.empty.title')}</span>
          <span className='text-11px leading-17px text-t-tertiary'>{t('sessionList.remoteEmptyHint')}</span>
        </div>
      ) : (
        <div className='max-h-268px overflow-auto flex flex-col gap-2px'>
          {rows.map((host) => (
            <button
              key={host.sshHostId}
              type='button'
              disabled={busy}
              aria-busy={startingId === host.sshHostId}
              className='w-full px-6px py-6px rd-6px border-none bg-transparent outline-none flex items-center gap-8px text-left cursor-pointer transition-colors hover:bg-fill-3 active:bg-fill-4 disabled:cursor-default disabled:opacity-55 focus:outline-none focus-visible:bg-fill-3'
              onClick={() => void launch(host)}
            >
              <span className='size-24px shrink-0 rd-6px flex items-center justify-center bg-brand-light text-brand'>
                <Server theme='outline' size='14' fill='currentColor' />
              </span>
              <span className='min-w-0 flex-1'>
                <span className='block truncate text-13px leading-19px font-[500] text-t-primary'>{host.name}</span>
                <span className='block truncate font-mono text-11px leading-16px text-t-tertiary'>
                  {host.username}@{host.host}:{host.port}
                </span>
              </span>
            </button>
          ))}
        </div>
      )}

      {rows.length > 0 && <div className='my-2px h-1px bg-[var(--color-border-2)]' />}

      <button
        type='button'
        data-testid='remote-session-add-host'
        className='w-full h-30px px-6px rd-6px border-none bg-transparent outline-none flex items-center gap-6px cursor-pointer text-13px leading-none text-t-secondary transition-colors hover:bg-fill-3 hover:text-t-primary focus:outline-none focus-visible:bg-fill-3'
        onClick={openForm}
      >
        <Plus theme='outline' size='14' fill='currentColor' className='block leading-none shrink-0' />
        <span className='truncate'>{t('sessionList.remoteAddHost')}</span>
      </button>
    </div>
  );

  return (
    // One wrapper element = one action-grid cell, whatever the modal renders.
    <div className='min-w-0'>
      <Popover
        trigger='click'
        position='br'
        content={content}
        popupVisible={visible}
        onVisibleChange={handleVisibleChange}
        getPopupContainer={() => document.body}
      >
        <button
          type='button'
          data-testid='session-new-remote-entry'
          className={buttonClassName}
          aria-expanded={visible}
          aria-label={t('sessionList.newRemoteSession')}
        >
          <Plus
            theme='outline'
            size='15'
            fill='currentColor'
            className='block leading-none shrink-0'
            style={{ lineHeight: 0 }}
          />
          <span className='truncate min-w-0'>{t('sessionList.actionRemote')}</span>
        </button>
      </Popover>

      {/* Same form the settings host book uses. A host saved from here is one the
          user asked to work on, so it opens its session immediately. */}
      <SshHostFormModal
        visible={formVisible}
        onClose={() => setFormVisible(false)}
        onSaved={(host) => {
          void mutate();
          if (host) void launch(host);
        }}
      />
    </div>
  );
};

export default RemoteSessionPopover;
