import { Collapse, Tooltip } from '@arco-design/web-react';
import { Info } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ExtensionMcpServerContribution } from '@/renderer/hooks/mcp/extensionCatalog';
import { iconColors } from '@/renderer/styles/colors';

interface ExtensionMcpServerItemProps {
  server: ExtensionMcpServerContribution;
  isCollapsed: boolean;
  onToggleCollapse: () => void;
}

/** Read-only presentation for an extension contribution, with no canonical MCP actions. */
const ExtensionMcpServerItem: React.FC<ExtensionMcpServerItemProps> = ({
  server,
  isCollapsed,
  onToggleCollapse,
}) => {
  const { t } = useTranslation();
  const hasDescription = Boolean(server.description);

  return (
    <Collapse
      activeKey={hasDescription && isCollapsed ? ['1'] : []}
      onChange={hasDescription ? onToggleCollapse : undefined}
      className='mb-4 [&_div.arco-collapse-item-header-title]:flex-1'
    >
      <Collapse.Item
        header={
          <div className='flex items-center gap-2'>
            <span>{server.name}</span>
            <Tooltip content={t('settings.mcpDisconnected')} position='top'>
              <span className='flex items-center cursor-default'>
                <Info theme='outline' fill={iconColors.secondary} className='h-[24px]' />
              </span>
            </Tooltip>
          </div>
        }
        name='1'
        disabled={!hasDescription}
        showExpandIcon={hasDescription}
        className='[&_div.arco-collapse-item-content-box]:py-3'
      >
        {hasDescription ? (
          <div className='text-13px leading-20px text-t-secondary whitespace-pre-wrap break-words'>
            {server.description}
          </div>
        ) : null}
      </Collapse.Item>
    </Collapse>
  );
};

export default ExtensionMcpServerItem;
