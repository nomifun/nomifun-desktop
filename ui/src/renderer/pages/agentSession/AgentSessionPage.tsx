import { agentPlatform, type IAgentSessionCapabilityState, type IAgentSessionObservation } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { ForkAgentSessionRequest } from '@/common/types/agentPlatform';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { Alert, Button, Input, Popconfirm, Spin, Tag } from '@arco-design/web-react';
import { ArrowLeft, Branch, Delete, PlayOne, Refresh } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';
import { projectionCards } from './model';
import SessionInspector from './SessionInspector';
import SessionProjectionCard from './SessionProjectionCard';
import styles from './AgentSessionPage.module.css';

const requestKey = (): string =>
  `agent-session-ui-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;

const AgentSessionPage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { agentSessionId = '' } = useParams();
  const [observation, setObservation] = useState<IAgentSessionObservation | null>(null);
  const [capabilities, setCapabilities] = useState<IAgentSessionCapabilityState | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<'turn' | 'fork' | 'delete' | null>(null);
  const [input, setInput] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [deleted, setDeleted] = useState(false);

  const load = useCallback(async () => {
    if (!agentSessionId || deleted) return;
    try {
      const [nextObservation, nextCapabilities] = await Promise.all([
        agentPlatform.sessions.get.invoke({ agent_session_id: agentSessionId }),
        agentPlatform.sessions.capabilities.invoke({ agent_session_id: agentSessionId }),
      ]);
      setObservation(nextObservation);
      setCapabilities(nextCapabilities);
      setError(null);
    } catch (loadError) {
      if (isBackendHttpError(loadError) && loadError.code === 'SESSION_DELETED') {
        setDeleted(true);
        setObservation(null);
        setCapabilities(null);
      } else {
        setError(String(loadError));
      }
    } finally {
      setLoading(false);
    }
  }, [agentSessionId, deleted]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!observation || !['opening', 'running'].includes(observation.head.status)) return;
    const timer = window.setInterval(() => void load(), 1500);
    return () => window.clearInterval(timer);
  }, [load, observation]);

  const cards = useMemo(
    () => projectionCards(observation?.messages ?? []),
    [observation?.messages]
  );

  const sendTurn = useCallback(async () => {
    if (!input.trim() || !observation) return;
    setBusy('turn');
    try {
      await agentPlatform.sessions.createTurn.invoke({
        agent_session_id: observation.session.agent_session_id,
        request: { input: { content: input.trim() }, idempotency_key: requestKey() },
      });
      setInput('');
      await load();
    } catch (turnError) {
      if (isBackendHttpError(turnError) && turnError.code === 'SESSION_DELETED') setDeleted(true);
      else setError(String(turnError));
    } finally {
      setBusy(null);
    }
  }, [input, load, observation]);

  const forkSession = useCallback(async () => {
    if (!observation) return;
    const request: ForkAgentSessionRequest =
      observation.continuation?.fork_request ?? {
        target_agent_binding: observation.session.agent_binding,
        parent_through_seq: observation.head.last_seq,
      };
    setBusy('fork');
    try {
      const result = await agentPlatform.sessions.fork.invoke({
        agent_session_id: observation.session.agent_session_id,
        request,
      });
      void navigate(`/agent-sessions/${result.child_agent_session_id}`);
    } catch (forkError) {
      if (isBackendHttpError(forkError) && forkError.code === 'SESSION_DELETED') setDeleted(true);
      else setError(String(forkError));
    } finally {
      setBusy(null);
    }
  }, [navigate, observation]);

  const deleteSession = useCallback(async () => {
    if (!observation) return;
    setBusy('delete');
    try {
      await agentPlatform.sessions.delete.invoke({
        agent_session_id: observation.session.agent_session_id,
      });
      setDeleted(true);
      setObservation(null);
      setCapabilities(null);
    } catch (deleteError) {
      if (isBackendHttpError(deleteError) && deleteError.code === 'SESSION_DELETED') setDeleted(true);
      else setError(String(deleteError));
    } finally {
      setBusy(null);
    }
  }, [observation]);

  if (deleted) {
    return (
      <HubPageShell title={t('agentSettings.session.deletedTitle')} maxWidthClass='md:max-w-900px'>
        <div className={styles.deletedState}>
          <Delete theme='outline' size='28' />
          <h2>SESSION_DELETED</h2>
          <p>{t('agentSettings.session.deletedBody')}</p>
          <Button type='primary' onClick={() => void navigate('/settings/agent-presets')}>
            {t('agentSettings.session.backToSettings')}
          </Button>
        </div>
      </HubPageShell>
    );
  }

  return (
    <HubPageShell
      title={observation?.session.metadata.title || t('agentSettings.session.title')}
      subtitle={agentSessionId}
      maxWidthClass='md:max-w-1400px'
      toolbar={
        <div className={styles.toolbar}>
          <Button type='text' icon={<ArrowLeft size='15' />} onClick={() => void navigate('/settings/agent-presets')}>
            {t('agentSettings.session.backToSettings')}
          </Button>
          <Button icon={<Refresh size='14' />} onClick={() => void load()}>{t('agentSettings.actions.retry')}</Button>
          <Button loading={busy === 'fork'} icon={<Branch size='14' />} onClick={() => void forkSession()}>
            {t('agentSettings.session.fork')}
          </Button>
          <Popconfirm title={t('agentSettings.session.deleteConfirm')} onOk={() => void deleteSession()}>
            <Button status='danger' loading={busy === 'delete'} icon={<Delete size='14' />}>
              {t('agentSettings.session.delete')}
            </Button>
          </Popconfirm>
        </div>
      }
    >
      {error && <Alert type='error' showIcon content={error} className={styles.error} />}
      {loading && !observation ? (
        <div className={styles.loading}><Spin /><span>{t('agentSettings.session.loading')}</span></div>
      ) : observation ? (
        <div className={styles.sessionGrid}>
          <main className={styles.transcript}>
            <div className={styles.statusBar}>
              <Tag color={observation.head.status === 'ready' ? 'green' : 'blue'}>{observation.head.status}</Tag>
              <span>{t('agentSettings.session.generation')}: {observation.head.active_set_generation}</span>
              <span>{t('agentSettings.session.lastSeq')}: {observation.head.last_seq}</span>
            </div>
            {observation.continuation?.requires_explicit_fork && (
              <Alert
                type='warning'
                showIcon
                title='SNAPSHOT_EXECUTOR_UNAVAILABLE'
                content={t('agentSettings.session.continuationRequired')}
              />
            )}
            <div className={styles.cardList}>
              {cards.map((card) => <SessionProjectionCard key={card.id} card={card} />)}
              {cards.length === 0 && <div className={styles.empty}>{t('agentSettings.session.empty')}</div>}
            </div>
            <div className={styles.composer}>
              <Input.TextArea
                value={input}
                disabled={observation.head.status !== 'ready'}
                placeholder={t('agentSettings.session.inputPlaceholder')}
                autoSize={{ minRows: 2, maxRows: 6 }}
                onChange={setInput}
              />
              <Button
                type='primary'
                icon={<PlayOne size='15' />}
                loading={busy === 'turn'}
                disabled={observation.head.status !== 'ready' || !input.trim()}
                onClick={() => void sendTurn()}
              >
                {t('agentSettings.session.send')}
              </Button>
            </div>
          </main>
          <SessionInspector observation={observation} capabilities={capabilities} />
        </div>
      ) : null}
    </HubPageShell>
  );
};

export default AgentSessionPage;
