import type { ConversationId } from '@/common/types/ids';
import { parseError } from '@/common/utils';
import { fetchConversationExportMessages } from '@/renderer/utils/chat/conversationExportData';
import { ipcBridge } from '@/common';
import type { TMessage } from '@/common/chat/chatLib';
import type { TChatConversation } from '@/common/config/storage';
import { Button } from '@arco-design/web-react';
import type { SlashCommandMenuItem } from '@/renderer/components/chat/SlashCommandMenu';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import {
  type ExportTranscriptLabels,
  buildConversationExportText,
  buildDefaultExportFileName,
  getDefaultExportFileNameSource,
  joinFilePath,
  normalizeExportFileName,
  resolveExportBaseDirectory,
} from '@/renderer/utils/chat/conversationExport';
import { copyText } from '@/renderer/utils/ui/clipboard';
import type { ArcoMessageInstance } from '@/renderer/utils/ui/useArcoMessage';
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from 'react';

type ExportFlowStep = 'closed' | 'menu' | 'filename';
class ExportFlowCanceledError extends Error {}

type MessageApi = Partial<Pick<ArcoMessageInstance, 'success' | 'error'>>;

type UseConversationExportOptions = {
  conversation_id?: ConversationId;
  workspace?: string;
  t: (key: string, options?: Record<string, unknown>) => string;
  messageApi: MessageApi;
};

type UseConversationExportResult = {
  step: ExportFlowStep;
  activeIndex: number;
  filename: string;
  loading: boolean;
  menuItems: SlashCommandMenuItem[];
  isOpen: boolean;
  pathPreview: string;
  openExportFlow: () => Promise<void>;
  closeExportFlow: () => void;
  showMenu: () => void;
  setFilename: (value: string) => void;
  setActiveIndex: (value: number) => void;
  onSelectMenuItem: (key: string) => void;
  handleKeyDown: (event: ReactKeyboardEvent) => boolean;
  submitFilename: () => Promise<void>;
};

export function useConversationExport(options: UseConversationExportOptions): UseConversationExportResult {
  const { conversation_id, workspace, t, messageApi } = options;
  const [step, setStep] = useState<ExportFlowStep>('closed');
  const [activeIndex, setActiveIndex] = useState(0);
  const [filename, setFilename] = useState('');
  const [loading, setLoading] = useState(false);
  const conversationRef = useRef<TChatConversation | null>(null);
  const baseDirectoryRef = useRef('');
  const messagesRef = useRef<TMessage[] | null>(null);
  const flowRevisionRef = useRef(0);
  const busyRef = useRef(false);
  const transcriptLabels = useMemo<ExportTranscriptLabels>(
    () => ({
      conversation: t('messages.export.conversationLabel'),
      conversation_id: t('messages.export.conversation_idLabel'),
      exportedAt: t('messages.export.exportedAtLabel'),
      type: t('messages.export.typeLabel'),
      noMessages: t('messages.export.noMessages'),
      user: t('messages.export.userLabel'),
      assistant: t('messages.export.assistantLabel'),
      system: t('messages.export.systemLabel'),
    }),
    [t]
  );

  const closeExportFlow = useCallback(() => {
    flowRevisionRef.current += 1;
    busyRef.current = false;
    conversationRef.current = null;
    messagesRef.current = null;
    baseDirectoryRef.current = '';
    setStep('closed');
    setActiveIndex(0);
    setLoading(false);
  }, []);

  useEffect(() => {
    closeExportFlow();
    return () => { flowRevisionRef.current += 1; };
  }, [closeExportFlow, conversation_id]);

  const showMenu = useCallback(() => {
    setStep('menu');
    setActiveIndex(0);
  }, []);

  const loadConversation = useCallback(async (revision: number): Promise<TChatConversation | null> => {
    if (!conversation_id) {
      return null;
    }
    if (conversationRef.current?.id === conversation_id) {
      return conversationRef.current;
    }

    const conversation = await getConversationOrNull(conversation_id);
    if (flowRevisionRef.current !== revision) throw new ExportFlowCanceledError();
    conversationRef.current = conversation;
    messagesRef.current = null;
    return conversation;
  }, [conversation_id]);

  const loadTranscript = useCallback(async (revision: number): Promise<string | null> => {
    if (!conversation_id) {
      return null;
    }
    const conversation = await loadConversation(revision);
    if (!conversation) {
      return null;
    }

    const messages = messagesRef.current ?? await fetchConversationExportMessages(conversation_id, () => {
      if (flowRevisionRef.current !== revision) throw new ExportFlowCanceledError();
    });
    if (flowRevisionRef.current !== revision) throw new ExportFlowCanceledError();
    messagesRef.current = messages;
    return buildConversationExportText(conversation, messages, transcriptLabels);
  }, [conversation_id, loadConversation, transcriptLabels]);

  const openExportFlow = useCallback(async () => {
    if (!conversation_id) {
      messageApi.error?.(t('messages.export.unavailable'));
      return;
    }

    if (busyRef.current) return;
    const revision = ++flowRevisionRef.current;
    busyRef.current = true;
    setLoading(true);
    const assertActive = () => {
      if (flowRevisionRef.current !== revision) throw new ExportFlowCanceledError();
    };
    try {
      conversationRef.current = null;
      messagesRef.current = null;
      const conversation = await loadConversation(revision);
      if (!conversation) {
        messageApi.error?.(t('messages.export.unavailable'));
        return;
      }

      let desktopPath = '';
      if (!workspace?.trim()) {
        try {
          desktopPath = await ipcBridge.application.getPath.invoke({ name: 'desktop' });
        } catch {
          desktopPath = '';
        }
      }

      assertActive();
      baseDirectoryRef.current = resolveExportBaseDirectory(workspace, desktopPath);
      const messages = await fetchConversationExportMessages(conversation_id, assertActive);
      messagesRef.current = messages;
      setFilename(
        buildDefaultExportFileName(conversation.id, getDefaultExportFileNameSource(conversation, messages))
      );
      setActiveIndex(0);
      setStep('menu');
    } catch (error) {
      if (flowRevisionRef.current !== revision || error instanceof ExportFlowCanceledError) return;
      console.error('[useConversationExport] Failed to open export flow:', error);
      messageApi.error?.(`${t('messages.export.prepareFailed')}: ${parseError(error)}`);
    } finally {
      if (flowRevisionRef.current === revision) {
        busyRef.current = false;
        setLoading(false);
      }
    }
  }, [conversation_id, loadConversation, messageApi, t, workspace]);

  const handleCopy = useCallback(async () => {
    if (busyRef.current) return;
    const revision = flowRevisionRef.current;
    busyRef.current = true;
    try {
      setLoading(true);
      const transcript = await loadTranscript(revision);
      if (!transcript) {
        messageApi.error?.(t('messages.export.unavailable'));
        closeExportFlow();
        return;
      }
      await copyText(transcript);
      if (flowRevisionRef.current !== revision) return;
      messageApi.success?.(t('messages.export.copySuccess'));
      closeExportFlow();
    } catch (error) {
      if (flowRevisionRef.current !== revision || error instanceof ExportFlowCanceledError) return;
      console.error('[useConversationExport] Failed to copy export:', error);
      messageApi.error?.(`${t('messages.export.copyFailed')}: ${parseError(error)}`);
    } finally {
      if (flowRevisionRef.current === revision) {
        busyRef.current = false;
        setLoading(false);
      }
    }
  }, [closeExportFlow, loadTranscript, messageApi, t]);

  const handleSave = useCallback(async () => {
    if (busyRef.current) return;
    const revision = flowRevisionRef.current;
    busyRef.current = true;
    try {
      setLoading(true);
      const transcript = await loadTranscript(revision);
      if (!transcript) {
        messageApi.error?.(t('messages.export.unavailable'));
        closeExportFlow();
        return;
      }

      const normalizedFileName = normalizeExportFileName(filename);
      const targetPath = joinFilePath(baseDirectoryRef.current, normalizedFileName);
      const success = await ipcBridge.fs.writeFile.invoke({
        path: targetPath,
        data: transcript,
      });
      if (flowRevisionRef.current !== revision) return;

      if (!success) {
        messageApi.error?.(t('messages.export.saveFailed'));
        return;
      }

      messageApi.success?.({
        content: (
          <div className='flex flex-col gap-8px'>
            <div>{t('messages.export.saveSuccess', { path: targetPath })}</div>
            <div className='flex justify-end'>
              <Button
                size='mini'
                type='text'
                onClick={() => {
                  void copyText(targetPath)
                    .then(() => {
                      messageApi.success?.(t('common.copySuccess'));
                    })
                    .catch(() => {
                      messageApi.error?.(t('common.copyFailed'));
                    });
                }}
              >
                {t('messages.copy')}
              </Button>
            </div>
          </div>
        ),
        duration: 5000,
      });
      closeExportFlow();
    } catch (error) {
      if (flowRevisionRef.current !== revision || error instanceof ExportFlowCanceledError) return;
      console.error('[useConversationExport] Failed to save export:', error);
      messageApi.error?.(`${t('messages.export.saveFailed')}: ${parseError(error)}`);
    } finally {
      if (flowRevisionRef.current === revision) {
        busyRef.current = false;
        setLoading(false);
      }
    }
  }, [closeExportFlow, filename, loadTranscript, messageApi, t]);

  const onSelectMenuItem = useCallback(
    (key: string) => {
      if (busyRef.current) {
        return;
      }
      if (key === 'copy') {
        void handleCopy();
        return;
      }
      if (key === 'save') {
        setStep('filename');
      }
    },
    [handleCopy]
  );

  const submitFilename = useCallback(async () => {
    if (busyRef.current) {
      return;
    }
    if (!baseDirectoryRef.current) {
      messageApi.error?.(t('messages.export.unavailable'));
      return;
    }
    await handleSave();
  }, [handleSave, messageApi, t]);

  const menuItems = useMemo<SlashCommandMenuItem[]>(
    () => [
      {
        key: 'copy',
        label: t('messages.export.copyLabel'),
        description: t('messages.export.copyDescription'),
      },
      {
        key: 'save',
        label: t('messages.export.saveLabel'),
        description: t('messages.export.saveDescription'),
      },
    ],
    [t]
  );

  const handleKeyDown = useCallback(
    (event: ReactKeyboardEvent) => {
      if (step === 'closed') {
        return false;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        if (step === 'filename') {
          showMenu();
        } else {
          closeExportFlow();
        }
        return true;
      }

      if (step === 'menu') {
        if (event.key === 'ArrowDown') {
          event.preventDefault();
          setActiveIndex((prev) => (prev + 1) % menuItems.length);
          return true;
        }
        if (event.key === 'ArrowUp') {
          event.preventDefault();
          setActiveIndex((prev) => (prev - 1 + menuItems.length) % menuItems.length);
          return true;
        }
        if (event.key === 'Enter' && !event.shiftKey) {
          event.preventDefault();
          const activeMenuItem = menuItems[activeIndex] ?? menuItems[0];
          if (activeMenuItem) {
            onSelectMenuItem(activeMenuItem.key);
          }
          return true;
        }
        return false;
      }

      if (step === 'filename' && event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        void submitFilename();
        return true;
      }

      return false;
    },
    [activeIndex, closeExportFlow, menuItems, onSelectMenuItem, showMenu, step, submitFilename]
  );

  return {
    step,
    activeIndex,
    filename,
    loading,
    menuItems,
    isOpen: step !== 'closed',
    pathPreview: baseDirectoryRef.current
      ? joinFilePath(baseDirectoryRef.current, normalizeExportFileName(filename))
      : normalizeExportFileName(filename),
    openExportFlow,
    closeExportFlow,
    showMenu,
    setFilename,
    setActiveIndex,
    onSelectMenuItem,
    handleKeyDown,
    submitFilename,
  };
}
