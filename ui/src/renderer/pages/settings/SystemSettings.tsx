/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useLocation } from 'react-router-dom';
import SystemModalContent from '@/renderer/components/settings/SettingsModal/contents/SystemModalContent';
import AboutModalContent from '@/renderer/components/settings/SettingsModal/contents/AboutModalContent';
import BrowserUseSettingsContent from '@/renderer/components/settings/SettingsModal/contents/BrowserUseSettingsContent';
import ComputerUseSettingsContent from '@/renderer/components/settings/SettingsModal/contents/ComputerUseSettingsContent';
import SettingsPageWrapper from './components/SettingsPageWrapper';

const SystemSettings: React.FC = () => {
  const location = useLocation();
  const isAboutPage = location.pathname === '/settings/about';
  const isBrowserUsePage = location.pathname === '/settings/browser-use';
  const isComputerUsePage = location.pathname === '/settings/computer-use';

  const content = (() => {
    if (isAboutPage) return <AboutModalContent />;
    if (isBrowserUsePage) return <BrowserUseSettingsContent />;
    if (isComputerUsePage) return <ComputerUseSettingsContent />;
    return <SystemModalContent />;
  })();

  return (
    <SettingsPageWrapper contentClassName={isAboutPage ? 'max-w-640px' : undefined}>
      {content}
    </SettingsPageWrapper>
  );
};

export default SystemSettings;
