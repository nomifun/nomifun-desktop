import type { ConversationId } from '@/common/types/ids';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { ArrowLeft, Refresh } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import ReadOnlyConversationView from '@/renderer/pages/conversation/execution/ReadOnlyConversationView';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { appErrorMessage } from './appHostingModel';
import styles from './AppsPage.module.css';

type Props = {
  appId: string;
  sessionId: ConversationId;
  appError: unknown;
  retrying?: boolean;
  onRetry: () => void;
};

/** An explicit App Session link can retain readable canonical history without a registry record. */
export default function UnavailableAppSessionViewer({ appId, sessionId, appError, retrying, onRetry }: Props) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data: conversation, error, isLoading } = useSWR(`conversation/${sessionId}`, () => getConversationOrNull(sessionId), {
    shouldRetryOnError: false,
  });
  const owned = conversation?.id === sessionId && conversation.app_project_id === appId
    && conversation.type === 'nomi' && conversation.session_purpose !== 'plugin_authoring';
  const agentName = conversation?.agent_snapshot?.preset_name || t('appHosting.project.retainedAgent');
  return <main className={styles.workspace} aria-label={t('appHosting.project.unavailableTitle')}>
    <header className={styles.workspaceHeader}>
      <Button type='text' icon={<ArrowLeft />} aria-label={t('appHosting.workspace.back')} onClick={() => void navigate('/apps')} />
      <h1>{owned ? conversation.name : t('appHosting.project.unavailableTitle')}</h1>
      <Button size='small' icon={<Refresh />} loading={retrying} onClick={onRetry}>{t('appHosting.project.retryApp')}</Button>
    </header>
    <div className='px-16px py-12px flex flex-col gap-8px'>
      <Alert type='warning' content={t('appHosting.project.unavailableHint')} />
      <p className={styles.muted}>{appErrorMessage(appError)}</p>
      {owned && <p className={styles.muted}>{agentName}</p>}
    </div>
    <div className={`${styles.projectConversation} flex flex-col`}>
      {isLoading && !conversation ? <Spin /> : error && !conversation ? <Alert type='error' content={appErrorMessage(error)} />
        : !owned ? <Alert type='error' content={t('appHosting.workspace.sessionMismatch')} />
          : <ReadOnlyConversationView conversation={conversation} agent_name={agentName} />}
    </div>
  </main>;
}
