import type { IMcpServer } from '@/common/config/storage';
import { Alert, Button } from '@arco-design/web-react';
import React, { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import CodeMirror from '@uiw/react-codemirror';
import { json } from '@codemirror/lang-json';
import { Link } from '@icon-park/react';
import { useThemeContext } from '@/renderer/hooks/context/ThemeContext';
import NomiModal from '@/renderer/components/base/NomiModal';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { parseMcpJsonImport } from '../ToolsSettings/mcpJsonImport';
import { toImportableMcpServer } from '../ToolsSettings/mcpImportUtils';
import { getMcpApiKeyUrl } from '@/renderer/hooks/mcp/mcpAuthConfig';
import { openExternalUrl } from '@/renderer/utils/platform';

interface JsonImportModalProps {
  visible: boolean;
  server?: IMcpServer;
  onCancel: () => void;
  onSubmit: (server: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => Promise<void> | void;
  onBatchImport?: (
    servers: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>[]
  ) => Promise<IMcpServer[] | void> | IMcpServer[] | void;
}

interface ValidationResult {
  isValid: boolean;
  errorMessage?: string;
}

const validateEditServerNames = (
  currentName: string,
  serverKeys: string[],
  t: (key: string, options?: Record<string, string>) => string
): ValidationResult => {
  if (serverKeys.length !== 1) {
    return {
      isValid: false,
      errorMessage: t('settings.mcpEditNameLocked', { name: currentName }),
    };
  }

  if (serverKeys[0] !== currentName) {
    return {
      isValid: false,
      errorMessage: t('settings.mcpEditNameLocked', { name: currentName }),
    };
  }

  return { isValid: true };
};

const JsonImportModal: React.FC<JsonImportModalProps> = ({ visible, server, onCancel, onSubmit, onBatchImport }) => {
  const { t } = useTranslation();
  const { theme } = useThemeContext();
  const [jsonInput, setJsonInput] = useState('');
  const [copyStatus, setCopyStatus] = useState<'idle' | 'success' | 'error'>('idle');
  const [submitting, setSubmitting] = useState(false);
  const [validation, setValidation] = useState<ValidationResult>({ isValid: true });
  const apiKeyUrl = server ? getMcpApiKeyUrl(server.transport) : null;
  const handleOpenApiKeyUrl = useCallback(() => {
    if (!apiKeyUrl) return;
    void openExternalUrl(apiKeyUrl).catch((error) => {
      console.error('Failed to open MCP API page:', error);
    });
  }, [apiKeyUrl]);

  /**
   * JSON语法校验
   */
  const validateJsonSyntax = useCallback(
    (input: string): ValidationResult => {
      if (!input.trim()) {
        return { isValid: true }; // 空值视为有效
      }

      try {
        JSON.parse(input);
        return { isValid: true };
      } catch (error) {
        return {
          isValid: false,
          errorMessage: error instanceof SyntaxError ? error.message : t('settings.mcpJsonFormatError'),
        };
      }
    },
    [t]
  );

  // 监听 jsonInput 变化，实时更新校验结果
  React.useEffect(() => {
    setValidation(validateJsonSyntax(jsonInput));
  }, [jsonInput, validateJsonSyntax]);

  // 当编辑现有服务器时，预填充JSON数据
  React.useEffect(() => {
    if (visible && server) {
      // 优先使用存储的original_json，如果没有则生成JSON配置
      if (server.original_json) {
        setJsonInput(server.original_json);
      } else {
        // 兼容没有original_json的旧数据，生成JSON配置
        const serverConfig = {
          mcpServers: {
            [server.name]: {
              description: server.description,
              ...(server.transport.type === 'stdio'
                ? {
                    command: server.transport.command,
                    args: server.transport.args || [],
                    env: server.transport.env || {},
                  }
                : {
                    type: server.transport.type,
                    url: server.transport.url,
                    ...(server.transport.headers && { headers: server.transport.headers }),
                  }),
            },
          },
        };
        setJsonInput(JSON.stringify(serverConfig, null, 2));
      }
    } else if (visible && !server) {
      // 新建模式下清空JSON输入
      setJsonInput('');
    }
  }, [visible, server]);

  const handleSubmit = async () => {
    if (submitting) {
      return;
    }

    // Re-validate at submit time to guard against race between useEffect validation and click
    let config: unknown;
    try {
      config = JSON.parse(jsonInput);
    } catch {
      setValidation({ isValid: false, errorMessage: t('settings.mcpJsonFormatError') });
      return;
    }

    const parseResult = parseMcpJsonImport(config);
    if (parseResult.isValid === false) {
      setValidation({ isValid: false, errorMessage: t(parseResult.errorKey) });
      return;
    }

    const parsedServers = parseResult.servers;
    if (server) {
      const editNameValidation = validateEditServerNames(
        server.name,
        parsedServers.map((parsedServer) => parsedServer.name),
        t
      );
      if (!editNameValidation.isValid) {
        setValidation(editNameValidation);
        return;
      }
    }

    setSubmitting(true);

    // 如果有多个服务器，使用批量导入
    if (parsedServers.length > 1 && onBatchImport) {
      const serversToImport = parsedServers.map((parsedServer) =>
        toImportableMcpServer(
          parsedServer,
          JSON.stringify({ mcpServers: { [parsedServer.name]: parsedServer.originalConfig } }, null, 2),
          true
        )
      );

      try {
        await onBatchImport(serversToImport);
        onCancel();
      } finally {
        setSubmitting(false);
      }
      return;
    }

    // 单个服务器导入
    try {
      await onSubmit(toImportableMcpServer(parsedServers[0], jsonInput, server?.enabled ?? true));
      onCancel();
    } finally {
      setSubmitting(false);
    }
  };

  if (!visible) return null;

  return (
    <NomiModal
      visible={visible}
      onCancel={onCancel}
      onOk={handleSubmit}
      okButtonProps={{ disabled: !validation.isValid || submitting, loading: submitting }}
      header={{ title: server ? t('settings.mcpEditServer') : t('settings.mcpImportFromJSON'), showClose: true }}
      style={{ width: 600, height: 450 }}
      contentStyle={{
        borderRadius: 16,
        padding: '24px',
        background: 'var(--dialog-fill-0)',
        overflow: 'auto',
        height: 420 - 80,
      }} // 与“添加模型”弹窗保持统一尺寸 / Keep same size as Add Model modal
    >
      <div className='space-y-12px'>
        <div>
          <div className='mb-2 flex items-center justify-between gap-2'>
            <div className='text-sm text-t-secondary'>{t('settings.mcpImportPlaceholder')}</div>
            {apiKeyUrl && (
              <Button
                size='mini'
                type='text'
                icon={<Link size={'14'} />}
                title={t('settings.mcpGetApi', { defaultValue: 'Get API' })}
                onClick={handleOpenApiKeyUrl}
              >
                {t('settings.mcpGetApi', { defaultValue: 'Get API' })}
              </Button>
            )}
          </div>
          {!validation.isValid && jsonInput.trim() && (
            <Alert
              className='mb-3'
              type='error'
              showIcon
              content={validation.errorMessage || t('settings.mcpJsonFormatError') || 'JSON format error'}
            />
          )}
          <div className='relative'>
            <CodeMirror
              value={jsonInput}
              height='300px'
              theme={theme}
              extensions={[json()]}
              onChange={(value: string) => setJsonInput(value)}
              placeholder={`{
  "mcpServers": {
    "weather": {
      "command": "uv",
      "args": ["--directory", "/path/to/weather", "run", "weather.py"],
      "description": "Weather information server"
    }
  }
}`}
              basicSetup={{
                lineNumbers: true,
                foldGutter: true,
                dropCursor: false,
                allowMultipleSelections: false,
              }}
              style={{
                fontSize: '13px',
                border: validation.isValid || !jsonInput.trim() ? '1px solid var(--bg-3)' : '1px solid var(--danger)',
                borderRadius: '6px',
                marginBottom: '20px',
                overflow: 'hidden',
              }}
              className='[&_.cm-editor]:rounded-[6px]'
            />
            {jsonInput && (
              <Button
                size='mini'
                type='outline'
                className='absolute top-2 right-2 z-10'
                onClick={() => {
                  void copyText(jsonInput)
                    .then(() => {
                      setCopyStatus('success');
                      setTimeout(() => setCopyStatus('idle'), 2000);
                    })
                    .catch((err) => {
                      console.error('Copy failed 复制失败:', err);
                      setCopyStatus('error');
                      setTimeout(() => setCopyStatus('idle'), 2000);
                    });
                }}
                style={{
                  backdropFilter: 'blur(4px)',
                }}
              >
                {copyStatus === 'success'
                  ? t('common.copySuccess')
                  : copyStatus === 'error'
                    ? t('common.copyFailed')
                    : t('common.copy')}
              </Button>
            )}
          </div>
        </div>

        <Alert
          type='info'
          showIcon
          content={
            <div>
              <div>{t('settings.mcpImportTips')}</div>
              <ul className='list-disc pl-5 mt-2 space-y-1 text-sm'>
                <li>{t('settings.mcpImportTip1')}</li>
                <li>{t('settings.mcpImportTip2')}</li>
                <li>{t('settings.mcpImportTip3')}</li>
              </ul>
            </div>
          }
        />
      </div>
    </NomiModal>
  );
};

export default JsonImportModal;
