import type { TChatConversation } from '@/common/config/storage';
import { browserStorageKey } from '@/common/utils/browserStorageKey';
import { Modal } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import ChatSlider from '@/renderer/pages/conversation/components/ChatSlider';
import { PreviewPanel, PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';

function Files({ conversation }: { conversation: TChatConversation }) {
  const { t } = useTranslation();
  const { isOpen, closePreview } = usePreviewContext();
  return <>
    <ChatSlider conversation={conversation} />
    <Modal visible={isOpen} footer={null} title={t('conversation.workspace.title')} onCancel={closePreview}
      style={{ width: 'calc(100% - 48px)', maxWidth: 1200, top: 24 }}
      unmountOnExit>
      <div style={{ height: 'calc(100vh - 184px)', overflow: 'hidden' }}><PreviewPanel /></div>
    </Modal>
  </>;
}

/** File navigation shares the existing editor, scoped to this canonical Session. */
export default function AppWorkspaceFiles({ conversation }: { conversation: TChatConversation }) {
  return <PreviewProvider persistNamespace={browserStorageKey('app-workspace-files', 'conversation', conversation.id)} subscribeGlobalOpen={false}>
    <Files conversation={conversation} />
  </PreviewProvider>;
}
