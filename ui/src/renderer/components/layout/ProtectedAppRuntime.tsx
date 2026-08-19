/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect } from 'react';
import { Navigate, Outlet, useNavigate } from 'react-router-dom';

import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import AppLoader from '@renderer/components/layout/AppLoader';
import { useAuth } from '@renderer/hooks/context/AuthContext';
import { useCompanionWindowsSync } from '@renderer/hooks/useCompanionWindowsSync';
import { useTrayLabels } from '@renderer/hooks/useTrayLabels';

/**
 * Authenticated application runtime shared by every protected product surface.
 *
 * This component deliberately owns no visible layout. It keeps application-wide
 * desktop effects alive and renders the selected child layout through Outlet,
 * so a protected surface is not forced to clone or mount the workbench shell.
 */
const ProtectedAppRuntime: React.FC = () => {
  const { status } = useAuth();

  if (status === 'checking') {
    return <AppLoader />;
  }

  if (status !== 'authenticated') {
    return <Navigate to='/login' replace />;
  }

  return (
    <>
      <CompanionNavigateListener />
      <CompanionWindowsSyncMount />
      <TrayLabelsMount />
      <Outlet />
    </>
  );
};

// Owns the native desktop-companion window set from the main window: reconciles one
// `companion-{companion_id}` Tauri window per enabled companion. Inert outside Tauri.
const CompanionWindowsSyncMount: React.FC = () => {
  useCompanionWindowsSync();
  return null;
};

// Keeps native system-tray labels (Show / Quit) in sync with the UI locale.
// Inert outside the Tauri desktop shell.
const TrayLabelsMount: React.FC = () => {
  useTrayLabels();
  return null;
};

// Routes navigation requests emitted by a desktop-companion window into the
// authenticated main-window router. Inert outside the Tauri desktop shell.
const CompanionNavigateListener: React.FC = () => {
  const navigate = useNavigate();

  useEffect(() => {
    if (!isTauriRuntime()) return;

    let unlisten: (() => void) | undefined;
    let disposed = false;
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen<string>('companion-navigate', (event) => {
        if (typeof event.payload === 'string' && event.payload.startsWith('/')) {
          void navigate(event.payload);
        }
      }).then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
    );

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate]);

  return null;
};

export default ProtectedAppRuntime;
