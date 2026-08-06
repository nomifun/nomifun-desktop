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
    return null;
  }

  return (
    <div className='space-y-3'>
      <div>
        <div className='space-y-2'>
          {server.tools.map((tool, index) => (
            // border-2 是颜色类（--bg-2），和这张卡片自己的 bg-2 同色，等于没有边框；
            // 卡片描边统一用 Arco 的 border-arco-2（--color-border-2）。
            // `border-2` is a colour (--bg-2) identical to this card's own bg-2.
            <div key={index} className='rounded-lg border border-solid border-arco-2 bg-2 px-4 py-3'>
              <div className='flex gap-4'>
                <div className='flex-shrink-0 min-w-0 w-1/3'>
                  <div className='break-words text-sm font-semibold text-t-primary'>{tool.name}</div>
                </div>
                <div className='flex-1 min-w-0'>
                  <Tooltip content={tool.description || t('settings.mcpNoDescription')}>
                    <div className='line-clamp-1 cursor-pointer text-xs leading-5 text-t-secondary'>
                      {tool.description || t('settings.mcpNoDescription')}
                    </div>
                  </Tooltip>
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

export default McpServerToolsList;
