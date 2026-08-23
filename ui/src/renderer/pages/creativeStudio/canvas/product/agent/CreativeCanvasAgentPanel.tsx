/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from '@/common/utils/uuidv7';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import React, {
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import {
  CreativeStudioAgentChatController,
  CreativeStudioAgentPanel,
  type CreativeStudioAgentChatPort,
  type CreativeStudioAgentMessage,
  type CreativeStudioAgentPanelLoadState,
  type CreativeStudioAgentSendInput,
  type CreativeStudioAgentSessionSummary,
  type CreativeStudioAgentTurnEvent,
  type CreativeStudioAgentView,
} from '../../../agent';
import {
  createNomiCreativeStudioAgentChatPort,
  type NomiCreativeStudioAgentSessionResolver,
} from '../../../agent/adapters';
import {
  createCreativeStudioAgentSessionResolver,
  createNomiCreativeStudioAgentSessionHttpPort,
} from '../../../agent/session';
import type { CreativeChatSessionReference } from '../../../domain';
import type { CreativeModelSelectionRef } from '../../../models';
import {
  classifyCreativeCanvasAgentHistory,
  createCreativeCanvasAgentSession,
  creativeCanvasAgentModelSelection,
  creativeCanvasAgentSessionWithAuthoritativeHistory,
  creativeCanvasAgentSessionWithPendingTurn,
  creativeCanvasAgentSessionWithoutPendingTurn,
  replaceCreativeCanvasAgentSession,
} from './model';
import {
  selectCreativeCanvasAgentContextNodes,
  serializeCreativeCanvasAgentModelInput,
  type CreativeCanvasAgentContextSnapshot,
} from './context';
import {
  CREATIVE_STUDIO_PLANNING_SKILLS,
  DEFAULT_CREATIVE_STUDIO_PLANNING_SKILL_IDS,
  isCreativeStudioPlanningSkillId,
} from './planningSkills';
import type { CreativeCanvasAgentOp } from './artifacts';
import {
  projectCreativeCanvasAgentProposals,
  type CreativeCanvasProposalOverride,
} from './proposalProjection';

export interface CreativeCanvasAgentPanelHandle {
  /** Stop an active exclusive turn and wait for its durable settlement before route exit. */
  prepareToLeave(): Promise<boolean>;
  /** Re-read durable history and proposal receipts after an external reload. */
  refreshAuthority(): void;
}

export interface CreativeCanvasAgentPanelProps {
  canvasId: string;
  hydrated: boolean;
  sessions: readonly CreativeChatSessionReference[];
  activeSessionId: string | null;
  planningContext: CreativeCanvasAgentContextSnapshot | null;
  disabled?: boolean;
  onPersist(
    sessions: readonly CreativeChatSessionReference[],
    activeSessionId: string | null
  ): Promise<void>;
  onApplyCanvasOps(
    assistantMessageId: string,
    ops: readonly CreativeCanvasAgentOp[]
  ): Promise<void>;
  onCollapse(): void;
  onOpenModelSettings?(): void;
  /** Test seam; production uses the authenticated owner-only HTTP resolver. */
  resolveSession?: NomiCreativeStudioAgentSessionResolver;
  /** Test seam; production uses NomiFun's Conversation REST/WebSocket runtime. */
  chatPort?: CreativeStudioAgentChatPort;
  createId?: () => string;
  now?: () => number;
}

interface AgentDocumentState {
  sessions: readonly CreativeChatSessionReference[];
  activeSessionId: string | null;
}

const copyHistory = (
  history: readonly CreativeStudioAgentMessage[]
): CreativeStudioAgentMessage[] => history.map((message) => ({ ...message }));

const sessionSummaries = (
  sessions: readonly CreativeChatSessionReference[],
  locale: string
): CreativeStudioAgentSessionSummary[] =>
  [...sessions]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .map((session) => ({
      id: session.id,
      title: session.title,
      messageCount: session.messageIds.length,
      updatedAtLabel: new Date(session.updatedAt).toLocaleString(locale, {
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
      }),
    }));

const sessionSignature = (session: CreativeChatSessionReference | undefined): string =>
  session
    ? JSON.stringify([
        session.id,
        session.messageIds,
        session.model,
        session.pendingTurn,
        session.updatedAt,
      ])
    : 'none';

const errorMessage = (error: unknown): string => {
  if (isBackendHttpError(error) && error.backendMessage.trim()) {
    return error.backendMessage.trim();
  }
  return error instanceof Error ? error.message : String(error);
};

const CreativeCanvasAgentPanel = React.forwardRef<
  CreativeCanvasAgentPanelHandle,
  CreativeCanvasAgentPanelProps
>((props, ref) => {
  const { t, i18n } = useTranslation();
  const createId = props.createId ?? uuidv7;
  const now = props.now ?? Date.now;
  const incomingSignature = JSON.stringify([props.activeSessionId, props.sessions]);
  const [documentState, setDocumentState] = useState<AgentDocumentState>(() => ({
    sessions: structuredClone([...props.sessions]),
    activeSessionId: props.activeSessionId,
  }));
  const documentRef = useRef(documentState);
  const mountedRef = useRef(true);
  const loadEpochRef = useRef(0);
  const locallySubmittedTurnKeyRef = useRef<string | null>(null);
  const locallyManagedActiveSignaturesRef = useRef<Set<string>>(new Set());
  const runningKeyRef = useRef<string | null>(null);
  const currentRunRef = useRef<Promise<void> | null>(null);
  const sendOperationRef = useRef<Promise<boolean> | null>(null);
  const documentMutationRef = useRef<Promise<void> | null>(null);
  const proposalApplyRef = useRef<Promise<void> | null>(null);
  const leaveEpochRef = useRef(0);
  const durableHistoryRef = useRef<readonly CreativeStudioAgentMessage[]>([]);

  const [view, setView] = useState<CreativeStudioAgentView>('chat');
  const [loadState, setLoadState] = useState<CreativeStudioAgentPanelLoadState>(
    props.hydrated ? 'ready' : 'loading'
  );
  const [messages, setMessages] = useState<readonly CreativeStudioAgentMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [selectedModel, setSelectedModel] = useState<CreativeModelSelectionRef | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [panelError, setPanelError] = useState<string | undefined>();
  const [loadRequest, setLoadRequest] = useState(0);
  const [excludedContextNodeIds, setExcludedContextNodeIds] = useState<readonly string[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<readonly string[]>([
    ...DEFAULT_CREATIVE_STUDIO_PLANNING_SKILL_IDS,
  ]);
  const [proposalOverrides, setProposalOverrides] = useState<
    Readonly<Record<string, CreativeCanvasProposalOverride>>
  >({});
  const [appliedProposalMessageIds, setAppliedProposalMessageIds] = useState<
    readonly string[]
  >([]);
  const isApplyingProposal = Object.values(proposalOverrides).some(
    (override) => override.state === 'applying'
  );

  const resolver = useMemo(
    () =>
      props.resolveSession ??
      createCreativeStudioAgentSessionResolver(
        createNomiCreativeStudioAgentSessionHttpPort()
      ),
    [props.resolveSession]
  );
  const chatPort = useMemo(
    () =>
      props.chatPort ??
      createNomiCreativeStudioAgentChatPort({
        resolveSession: resolver,
      }),
    [props.chatPort, resolver]
  );
  const controller = useMemo(() => new CreativeStudioAgentChatController(chatPort), [chatPort]);

  useEffect(() => {
    const next = {
      sessions: structuredClone([...props.sessions]),
      activeSessionId: props.activeSessionId,
    };
    documentRef.current = next;
    setDocumentState(next);
  }, [incomingSignature]);

  const activeSession = useMemo(
    () =>
      documentState.sessions.find(
        (session) => session.id === documentState.activeSessionId
      ),
    [documentState]
  );
  const activeSignature = sessionSignature(activeSession);
  const selectedContextSignature = JSON.stringify(
    props.planningContext?.selectedNodeIds ?? []
  );
  const contextItems = useMemo(
    () =>
      (props.planningContext?.nodes ?? [])
        .filter((node) => !excludedContextNodeIds.includes(node.id))
        .map((node) => ({
          id: node.id,
          label: node.label,
          type: node.type,
          selected: node.selected,
        })),
    [excludedContextNodeIds, props.planningContext?.nodes]
  );
  const proposalProjection = useMemo(() => {
    return projectCreativeCanvasAgentProposals(
      messages,
      proposalOverrides,
      appliedProposalMessageIds,
      t
    );
  }, [appliedProposalMessageIds, messages, proposalOverrides, t]);

  const translatedSkillOptions = useMemo(
    () =>
      CREATIVE_STUDIO_PLANNING_SKILLS.map((skill) => ({
        id: skill.id,
        label: t(skill.labelKey, {
          defaultValue:
            skill.id === 'creative-studio-canvas'
              ? '画布规划'
              : skill.id === 'creative-studio-organize'
                ? '整理布局'
                : '模板设计',
        }),
        description: t(skill.descriptionKey, {
          defaultValue:
            skill.id === 'creative-studio-canvas'
              ? '理解当前选择并提出安全的文本与结构操作。'
              : skill.id === 'creative-studio-organize'
                ? '调整现有节点的位置、尺寸与连接关系。'
                : '把创作目标整理成可人工确认的模板草案。',
        }),
      })),
    [t]
  );

  useEffect(() => {
    setExcludedContextNodeIds([]);
  }, [selectedContextSignature]);

  useEffect(() => {
    if (!activeSession?.pendingTurn) return;
    setSelectedSkillIds(
      activeSession.pendingTurn.skillIds.filter(isCreativeStudioPlanningSkillId)
    );
  }, [activeSession?.pendingTurn?.idempotencyKey]);

  const persistDocument = useCallback(
    async (
      sessions: readonly CreativeChatSessionReference[],
      activeSessionId: string | null
    ) => {
      if (documentMutationRef.current) {
        throw new Error('Creative Studio Agent document mutation is already in progress');
      }
      const previous = documentRef.current;
      const next = {
        sessions: structuredClone([...sessions]),
        activeSessionId,
      };
      documentRef.current = next;
      const operation = props.onPersist(next.sessions, next.activeSessionId);
      documentMutationRef.current = operation;
      try {
        await operation;
        if (mountedRef.current) setDocumentState(next);
      } catch (error) {
        documentRef.current = previous;
        throw error;
      } finally {
        if (documentMutationRef.current === operation) {
          documentMutationRef.current = null;
        }
      }
    },
    [props.onPersist]
  );

  const persistSession = useCallback(
    async (session: CreativeChatSessionReference, activeSessionId = session.id) => {
      const managedSignature = sessionSignature(session);
      locallyManagedActiveSignaturesRef.current.add(managedSignature);
      try {
        await persistDocument(
          replaceCreativeCanvasAgentSession(documentRef.current.sessions, session),
          activeSessionId
        );
      } catch (error) {
        locallyManagedActiveSignaturesRef.current.delete(managedSignature);
        throw error;
      }
    },
    [persistDocument]
  );

  const replaceRunningAssistant = useCallback(
    (
      assistantId: string,
      update: (message: Extract<CreativeStudioAgentMessage, { role: 'assistant' }>) => CreativeStudioAgentMessage
    ) => {
      setMessages((current) =>
        current.map((message) =>
          message.id === assistantId && message.role === 'assistant'
            ? update(message)
            : message
        )
      );
    },
    []
  );

  const runPersistedTurn = useCallback(
    async (
      session: CreativeChatSessionReference,
      history: readonly CreativeStudioAgentMessage[]
    ): Promise<void> => {
      const pending = session.pendingTurn;
      const model = creativeCanvasAgentModelSelection(session.model);
      if (!pending || !model) throw new Error('Creative Studio Agent turn is not durably fenced');
      if (runningKeyRef.current === pending.idempotencyKey) {
        await currentRunRef.current;
        return;
      }
      if (runningKeyRef.current) throw new Error('Creative Studio Agent already has an active turn');

      const transientUserId = createId();
      const transientAssistantId = createId();
      const transientUser: CreativeStudioAgentMessage = {
        id: transientUserId,
        role: 'user',
        status: 'complete',
        text: pending.prompt,
      };
      const transientAssistant: CreativeStudioAgentMessage = {
        id: transientAssistantId,
        role: 'assistant',
        status: 'running',
        text: '',
        activityLabel: t('creativeStudio.agent.connecting', {
          defaultValue: '正在连接 NomiFun Agent',
        }),
      };
      runningKeyRef.current = pending.idempotencyKey;
      setIsRunning(true);
      setLoadState('ready');
      setPanelError(undefined);
      setSelectedModel(model);
      setMessages([...copyHistory(history), transientUser, transientAssistant]);

      let reconciledHistory: readonly CreativeStudioAgentMessage[] | null = null;
      let terminalFailureObserved = false;
      const onEvent = (event: CreativeStudioAgentTurnEvent) => {
        if (!mountedRef.current) return;
        if (event.type === 'activity') {
          replaceRunningAssistant(transientAssistantId, (message) => ({
            ...message,
            status: 'running',
            activityLabel: event.label,
          }));
          return;
        }
        if (event.type === 'assistant-delta') {
          replaceRunningAssistant(transientAssistantId, (message) => ({
            ...message,
            status: 'running',
            text: message.text + event.delta,
          }));
          return;
        }
        if (event.type === 'history-reconciled') {
          classifyCreativeCanvasAgentHistory(session, event.history);
          reconciledHistory = copyHistory(event.history);
          durableHistoryRef.current = reconciledHistory;
          setMessages(reconciledHistory);
          return;
        }
        if (event.type === 'failed') {
          terminalFailureObserved = true;
          replaceRunningAssistant(transientAssistantId, (message) => ({
            id: message.id,
            role: 'assistant',
            status: 'failed',
            text: message.text,
            errorMessage: event.message,
          }));
        }
      };

      const operation = (async () => {
        try {
          const outcome = await controller.runTurn(
            {
              canvasId: props.canvasId,
              sessionId: session.id,
              idempotencyKey: pending.idempotencyKey,
              prompt: pending.prompt,
              modelInput: pending.modelInput,
              skillIds: pending.skillIds,
              model,
              history,
            },
            { onEvent }
          );
          if (outcome.state === 'completed') {
            if (!reconciledHistory) {
              throw new Error('Creative Studio Agent completed without authoritative history');
            }
            const completed = creativeCanvasAgentSessionWithAuthoritativeHistory(
              session,
              reconciledHistory,
              now()
            );
            await persistSession(completed);
            durableHistoryRef.current = reconciledHistory;
            if (mountedRef.current) {
              setMessages(copyHistory(reconciledHistory));
              setPanelError(undefined);
            }
            return;
          }
          if (outcome.state === 'stopped') {
            await persistSession(creativeCanvasAgentSessionWithoutPendingTurn(session, now()));
            if (mountedRef.current) {
              replaceRunningAssistant(transientAssistantId, (message) => ({
                id: message.id,
                role: 'assistant',
                status: 'stopped',
                text: message.text,
              }));
            }
            return;
          }

          const outcomeErrorMessage = errorMessage(outcome.error);
          if (terminalFailureObserved) {
            await persistSession(creativeCanvasAgentSessionWithoutPendingTurn(session, now()));
          } else if (mountedRef.current) {
            replaceRunningAssistant(transientAssistantId, (message) => ({
              id: message.id,
              role: 'assistant',
              status: 'failed',
              text: message.text,
              errorMessage: t('creativeStudio.agent.submitUnconfirmed', {
                message: outcomeErrorMessage,
                defaultValue: '提交结果尚未确认：{{message}}',
              }),
            }));
          }
          if (!terminalFailureObserved && mountedRef.current) {
            setPanelError(outcomeErrorMessage);
          }
        } catch (error) {
          if (mountedRef.current) {
            setPanelError(errorMessage(error));
            replaceRunningAssistant(transientAssistantId, (message) => ({
              id: message.id,
              role: 'assistant',
              status: 'failed',
              text: message.text,
              errorMessage: errorMessage(error),
            }));
          }
        } finally {
          runningKeyRef.current = null;
          currentRunRef.current = null;
          if (mountedRef.current) setIsRunning(false);
        }
      })();
      currentRunRef.current = operation;
      await operation;
    },
    [
      controller,
      createId,
      now,
      persistSession,
      props.canvasId,
      replaceRunningAssistant,
      t,
    ]
  );

  useEffect(() => {
    const epoch = ++loadEpochRef.current;
    const leaveEpoch = leaveEpochRef.current;
    const abort = new AbortController();
    if (!props.hydrated) {
      setLoadState('loading');
      return () => abort.abort();
    }
    if (!activeSession) {
      durableHistoryRef.current = [];
      setAppliedProposalMessageIds([]);
      setMessages([]);
      setSelectedModel(null);
      setLoadState('ready');
      setPanelError(undefined);
      return () => abort.abort();
    }
    const activeModel = creativeCanvasAgentModelSelection(activeSession.model);
    setSelectedModel(activeModel);
    if (locallyManagedActiveSignaturesRef.current.delete(activeSignature)) {
      // `persistSession` already owns both the display transition and the
      // durable Canvas mutation. Treat the controlled prop echo as an
      // acknowledgement, not as a request to replace the transcript from the
      // conversation authority again.
      setLoadState('ready');
      setPanelError(undefined);
      return () => abort.abort();
    }
    if (!activeSession.model) {
      durableHistoryRef.current = [];
      setAppliedProposalMessageIds([]);
      setMessages([]);
      setLoadState('ready');
      setPanelError(undefined);
      return () => abort.abort();
    }
    const activePendingKey = activeSession.pendingTurn?.idempotencyKey ?? null;
    if (
      runningKeyRef.current !== null ||
      (activePendingKey !== null &&
        locallySubmittedTurnKeyRef.current === activePendingKey)
    ) {
      // Local admission already owns the transient transcript. Re-resolving
      // the just-persisted pending session here would replace those optimistic
      // user/assistant rows with the older durable history, so later stream
      // deltas would have no rendered assistant row to update.
      setLoadState('ready');
      return () => abort.abort();
    }

    setLoadState('loading');
    setPanelError(undefined);
    setAppliedProposalMessageIds([]);
    void (async () => {
      try {
        const resolution = await resolver({
          canvasId: props.canvasId,
          sessionId: activeSession.id,
          model: activeModel!,
          pendingTurnIdempotencyKey: activeSession.pendingTurn?.idempotencyKey ?? null,
          signal: abort.signal,
        });
        if (abort.signal.aborted || epoch !== loadEpochRef.current) return;
        const authority = classifyCreativeCanvasAgentHistory(activeSession, resolution.history);
        setAppliedProposalMessageIds([...resolution.appliedProposalMessageIds]);
        if (authority !== 'current') {
          const reconciledHistory = copyHistory(resolution.history);
          durableHistoryRef.current = reconciledHistory;
          setMessages(reconciledHistory);
          const completed = creativeCanvasAgentSessionWithAuthoritativeHistory(
            activeSession,
            resolution.history,
            now()
          );
          await persistSession(completed);
          if (abort.signal.aborted || epoch !== loadEpochRef.current) return;
          setLoadState('ready');
          return;
        }
        durableHistoryRef.current = copyHistory(resolution.history);
        setMessages(durableHistoryRef.current);
        setLoadState('ready');
        if (activeSession.pendingTurn) {
          if (leaveEpoch !== leaveEpochRef.current) {
            setLoadState('failed');
            setPanelError(
              t('creativeStudio.agent.recoveryPaused', {
                defaultValue: 'Agent 恢复已暂停；请重试当前会话。',
              })
            );
            return;
          }
          await runPersistedTurn(activeSession, durableHistoryRef.current);
        }
      } catch (error) {
        if (abort.signal.aborted || epoch !== loadEpochRef.current) return;
        setLoadState('failed');
        setPanelError(errorMessage(error));
      }
    })();
    return () => abort.abort();
  }, [
    activeSignature,
    loadRequest,
    now,
    persistSession,
    props.canvasId,
    props.hydrated,
    resolver,
    runPersistedTurn,
  ]);

  const handleNewSession = useCallback(() => {
    if (
      isRunning ||
      proposalApplyRef.current ||
      sendOperationRef.current ||
      documentMutationRef.current
    ) return;
    void (async () => {
      try {
        const session = createCreativeCanvasAgentSession(
          createId(),
          now(),
          t('creativeStudio.agent.newConversation', {
            defaultValue: '新对话',
          })
        );
        await persistSession(session);
        durableHistoryRef.current = [];
        setMessages([]);
        setSelectedModel(null);
        setPanelError(undefined);
        setView('chat');
      } catch (error) {
        setPanelError(errorMessage(error));
      }
    })();
  }, [createId, isRunning, now, persistSession, t]);

  const handleSelectSession = useCallback(
    (sessionId: string) => {
      if (
        isRunning ||
        proposalApplyRef.current ||
        sendOperationRef.current ||
        documentMutationRef.current ||
        sessionId === documentRef.current.activeSessionId
      ) {
        setView('chat');
        return;
      }
      void persistDocument(documentRef.current.sessions, sessionId)
        .then(() => setView('chat'))
        .catch((error: unknown) => setPanelError(errorMessage(error)));
    },
    [isRunning, persistDocument]
  );

  const handleSend = useCallback(
    (input: CreativeStudioAgentSendInput) => {
      if (
        isRunning ||
        proposalApplyRef.current ||
        sendOperationRef.current ||
        documentMutationRef.current
      ) return;
      const planningContext = props.planningContext;
      if (
        !planningContext ||
        input.skillIds.length === 0 ||
        input.skillIds.length > 3 ||
        input.skillIds.some((skillId) => !isCreativeStudioPlanningSkillId(skillId))
      ) {
        setPanelError(
          t('creativeStudio.agent.skillSelectionRequired', {
            defaultValue: '请明确选择 1–3 个 Creative Studio 创作技能。',
          })
        );
        return;
      }
      const leaveEpoch = leaveEpochRef.current;
      setIsRunning(true);
      setPanelError(undefined);
      const operation = (async () => {
        let locallySubmittedTurnKey: string | null = null;
        try {
          const current =
            documentRef.current.sessions.find(
              (session) => session.id === documentRef.current.activeSessionId
            ) ??
            createCreativeCanvasAgentSession(
              createId(),
              now(),
              t('creativeStudio.agent.newConversation', {
                defaultValue: '新对话',
              })
            );
          const idempotencyKey = createId();
          const pending = creativeCanvasAgentSessionWithPendingTurn({
            session: current,
            model: input.model,
            idempotencyKey,
            prompt: input.prompt,
            modelInput: serializeCreativeCanvasAgentModelInput({
              prompt: input.prompt,
              context: selectCreativeCanvasAgentContextNodes(
                planningContext,
                input.contextNodeIds
              ),
              skillIds: input.skillIds,
            }),
            skillIds: input.skillIds,
            initialTitle: t('creativeStudio.agent.newConversation', {
              defaultValue: '新对话',
            }),
            now: now(),
          });
          locallySubmittedTurnKey = idempotencyKey;
          locallySubmittedTurnKeyRef.current = idempotencyKey;
          await persistSession(pending);
          if (!mountedRef.current || leaveEpoch !== leaveEpochRef.current) {
            await persistSession(
              creativeCanvasAgentSessionWithoutPendingTurn(pending, now())
            );
            return true;
          }
          setDraft('');
          await runPersistedTurn(pending, durableHistoryRef.current);
          return true;
        } catch (error) {
          if (mountedRef.current) setPanelError(errorMessage(error));
          return false;
        } finally {
          if (
            locallySubmittedTurnKey !== null &&
            locallySubmittedTurnKeyRef.current === locallySubmittedTurnKey
          ) {
            locallySubmittedTurnKeyRef.current = null;
          }
          if (mountedRef.current && !runningKeyRef.current) setIsRunning(false);
        }
      })();
      sendOperationRef.current = operation;
      void operation.finally(() => {
        if (sendOperationRef.current === operation) sendOperationRef.current = null;
      });
    },
    [
      createId,
      isRunning,
      now,
      persistSession,
      props.planningContext,
      runPersistedTurn,
      t,
    ]
  );

  const handleRetryLoad = useCallback(() => {
    loadEpochRef.current += 1;
    setLoadState('loading');
    setLoadRequest((current) => current + 1);
  }, []);

  const handleRetryMessage = useCallback(() => {
    const current = documentRef.current.sessions.find(
      (session) => session.id === documentRef.current.activeSessionId
    );
    if (
      !current?.pendingTurn ||
      isRunning ||
      proposalApplyRef.current ||
      sendOperationRef.current ||
      documentMutationRef.current
    ) return;
    void runPersistedTurn(current, durableHistoryRef.current);
  }, [isRunning, runPersistedTurn]);

  const handleStop = useCallback(() => {
    leaveEpochRef.current += 1;
    controller.stop();
  }, [controller]);

  const handleApplyProposal = useCallback(
    (messageId: string) => {
      const artifact = proposalProjection.artifacts.get(messageId);
      if (
        !artifact ||
        proposalOverrides[messageId] ||
        isRunning ||
        proposalApplyRef.current ||
        sendOperationRef.current ||
        documentMutationRef.current
      ) {
        return;
      }
      setProposalOverrides((current) => ({
        ...current,
        [messageId]: { state: 'applying' },
      }));
      const operation = (async () => {
        try {
          await props.onApplyCanvasOps(messageId, artifact.ops);
          if (!mountedRef.current) return;
          setProposalOverrides((current) => ({
            ...current,
            [messageId]: { state: 'applied' },
          }));
          setAppliedProposalMessageIds((current) =>
            current.includes(messageId) ? current : [...current, messageId]
          );
        } catch {
          if (!mountedRef.current) return;
          setProposalOverrides((current) => ({
            ...current,
            [messageId]: {
              state: 'failed',
              errorMessage: t('creativeStudio.agent.applyUnconfirmed', {
                defaultValue: '应用结果未确认；请检查页面提示并复核远端画布。',
              }),
            },
          }));
        }
      })();
      proposalApplyRef.current = operation;
      void operation.finally(() => {
        if (proposalApplyRef.current === operation) {
          proposalApplyRef.current = null;
        }
      });
    },
    [
      isRunning,
      proposalOverrides,
      proposalProjection.artifacts,
      props.onApplyCanvasOps,
      t,
    ]
  );

  useImperativeHandle(
    ref,
    () => ({
      refreshAuthority() {
        loadEpochRef.current += 1;
        setLoadState('loading');
        setLoadRequest((current) => current + 1);
      },
      async prepareToLeave() {
        leaveEpochRef.current += 1;
        if (controller.isRunning) controller.stop();
        const admittedSend = sendOperationRef.current;
        const documentMutation = documentMutationRef.current;
        const proposalApply = proposalApplyRef.current;
        const [sendSettled, mutationSettled, proposalSettled] = await Promise.all([
          admittedSend ?? Promise.resolve(true),
          documentMutation?.then(
            () => true,
            () => false
          ) ?? Promise.resolve(true),
          proposalApply?.then(
            () => true,
            () => false
          ) ?? Promise.resolve(true),
        ]);
        if (controller.isRunning) controller.stop();
        await currentRunRef.current;
        return sendSettled && mutationSettled && proposalSettled && !controller.isRunning;
      },
    }),
    [controller]
  );

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      leaveEpochRef.current += 1;
      controller.stop();
    };
  }, [controller]);

  const lockedModel = creativeCanvasAgentModelSelection(activeSession?.model ?? null);
  const model = lockedModel ?? selectedModel;

  return (
    <CreativeStudioAgentPanel
      view={view}
      loadState={loadState}
      sessions={sessionSummaries(
        documentState.sessions,
        i18n.resolvedLanguage ?? i18n.language
      )}
      activeSessionId={documentState.activeSessionId}
      messages={messages}
      proposals={proposalProjection.proposals}
      draft={draft}
      model={model}
      contextItems={contextItems}
      skillOptions={translatedSkillOptions}
      selectedSkillIds={selectedSkillIds}
      modelLocked={lockedModel !== null}
      isRunning={isRunning}
      errorMessage={panelError}
      disabled={props.disabled || isApplyingProposal}
      onViewChange={setView}
      onNewSession={handleNewSession}
      onSelectSession={handleSelectSession}
      onDraftChange={setDraft}
      onModelChange={(nextModel) => {
        if (!lockedModel) {
          setSelectedModel({
            providerId: nextModel.providerId,
            model: nextModel.model,
          });
        }
      }}
      onRemoveContextItem={(itemId) =>
        setExcludedContextNodeIds((current) =>
          current.includes(itemId) ? current : [...current, itemId]
        )
      }
      onToggleSkill={(skillId) => {
        if (!isCreativeStudioPlanningSkillId(skillId)) return;
        setSelectedSkillIds((current) => {
          if (current.includes(skillId)) {
            return current.length === 1
              ? current
              : current.filter((currentId) => currentId !== skillId);
          }
          return current.length >= 3 ? current : [...current, skillId];
        });
      }}
      onApplyProposal={handleApplyProposal}
      onSend={handleSend}
      onStop={handleStop}
      onCollapse={props.onCollapse}
      onRetryLoad={handleRetryLoad}
      onRetryMessage={handleRetryMessage}
      onOpenModelSettings={props.onOpenModelSettings}
    />
  );
});

CreativeCanvasAgentPanel.displayName = 'CreativeCanvasAgentPanel';

export default CreativeCanvasAgentPanel;
