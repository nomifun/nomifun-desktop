import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tooltip } from '@arco-design/web-react';
import type { IMcpServer } from '@/common/config/storage';

interface McpServerToolsListProps {
  server: IMcpServer;
}

const McpServerToolsList: React.FC<McpServerToolsListProps> = ({ server }) => {
  const { t } = useTranslation();

  if (!server.tools || server.tools.length === 0) {
    return (
      <div className='py-2px text-12px leading-20px text-t-tertiary' role='status'>
        {server.last_test_status === 'error'
          ? t('settings.mcpNoToolsAfterFailure')
          : t('settings.mcpNoToolsAvailable')}
      </div>
    );
  }

  return (
    <div className='relative grid grid-cols-2 gap-x-24px gap-y-0' data-testid='mcp-tool-grid'>
      <span
        aria-hidden='true'
        className='pointer-events-none absolute bottom-2px left-1/2 top-2px w-1px -translate-x-1/2 bg-[var(--color-border-2)]'
        data-testid='mcp-tool-column-divider'
      />
      {server.tools.map((tool) => (
        <div key={tool.name} className='flex min-w-0 items-center gap-6px py-2px' data-testid='mcp-tool-item'>
          <Tooltip content={tool.name}>
            <div className='max-w-[55%] min-w-0 flex-none truncate text-13px font-normal leading-20px text-t-primary'>
              {tool.name}
            </div>
          </Tooltip>
          <Tooltip content={tool.description || t('settings.mcpNoDescription')}>
            <div className='min-w-0 flex-1 truncate cursor-default text-12px leading-20px text-t-secondary'>
              {tool.description || t('settings.mcpNoDescription')}
            </div>
          </Tooltip>
        </div>
      ))}
    </div>
  );
};

export default McpServerToolsList;
