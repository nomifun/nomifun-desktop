/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import SettingsPageWrapper from '@renderer/pages/settings/components/SettingsPageWrapper';
import { useTranslation } from 'react-i18next';
import SshHostManagement from './SshHostManagement';

const SshHostSettings: React.FC = () => {
  const { t } = useTranslation();
  return (
    <SettingsPageWrapper contentClassName='max-w-1200px'>
      <header className='mb-18px'>
        <h1 className='m-0 text-20px font-600 leading-28px text-t-primary'>{t('ssh.title')}</h1>
        <p className='m-0 mt-4px text-12px leading-18px text-t-secondary'>{t('ssh.description')}</p>
      </header>
      <SshHostManagement />
    </SettingsPageWrapper>
  );
};

export default SshHostSettings;
