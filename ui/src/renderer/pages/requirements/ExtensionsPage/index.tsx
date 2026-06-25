/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { Remind, Robot } from '@icon-park/react';
import SegmentedTabs, { type SegmentedTabItem } from '@/renderer/components/base/SegmentedTabs';
import NotifyPanel from './NotifyPanel';
import AutoWorkPanel from './AutoWorkPanel';

type ExtTab = 'notify' | 'autowork';

const isExtTab = (value: string | null): value is ExtTab => value === 'notify' || value === 'autowork';

/**
 * ExtensionsPage — the requirements「扩展能力」section. A tabbed container over two
 * panels:
 *   - 通知 (NotifyPanel): notification channels + routing rules.
 *   - 自动执行 (AutoWorkPanel): tag→session AutoWork bindings overview.
 *
 * The active tab syncs to `?tab=notify|autowork` (default `notify`). This matches
 * the legacy-route redirects in Router (`/requirements/tag-sessions` & `/autowork`
 * → `?tab=autowork`; `/settings/webhook` & `/other` → `?tab=notify`).
 *
 * Renders inside RequirementsLayout's content pane, which already supplies the
 * centered, padded, scrollable wrapper — so this is just a thin header (the tab
 * toggle) + the active panel, mirroring WorkspacePage (no double-wrap).
 */
const ExtensionsPage: React.FC = () => {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();

  const tab: ExtTab = isExtTab(searchParams.get('tab')) ? (searchParams.get('tab') as ExtTab) : 'notify';

  const setTab = useCallback(
    (next: ExtTab) => {
      setSearchParams(
        (prev) => {
          const p = new URLSearchParams(prev);
          if (next === 'notify') p.delete('tab');
          else p.set('tab', next);
          return p;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  const tabItems: SegmentedTabItem[] = useMemo(
    () => [
      { key: 'notify', label: t('requirements.ext.notify'), icon: <Remind theme='outline' size='15' strokeWidth={3} /> },
      { key: 'autowork', label: t('requirements.ext.autowork'), icon: <Robot theme='outline' size='15' strokeWidth={3} /> },
    ],
    [t]
  );

  return (
    <div className='flex flex-col gap-16px'>
      <SegmentedTabs
        items={tabItems}
        activeKey={tab}
        onChange={(key) => setTab(isExtTab(key) ? key : 'notify')}
        size='sm'
      />
      {tab === 'notify' ? <NotifyPanel /> : <AutoWorkPanel />}
    </div>
  );
};

export default ExtensionsPage;
