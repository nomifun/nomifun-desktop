/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import SettingsPageWrapper from '@renderer/pages/settings/components/SettingsPageWrapper';
import { useTranslation } from 'react-i18next';
import SshHostManagement from './SshHostManagement';
import PageHeader from '@/renderer/components/layout/PageHeader';

const SshHostSettings: React.FC = () => {
  const { t } = useTranslation();
  return (
    <SettingsPageWrapper>
      <PageHeader title={t('ssh.title')} />
      <SshHostManagement />
    </SettingsPageWrapper>
  );
};

export default SshHostSettings;
