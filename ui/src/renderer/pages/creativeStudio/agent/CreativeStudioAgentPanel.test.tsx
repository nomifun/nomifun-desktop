/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import { parseProviderId } from '@/common/types/ids';

import CreativeStudioAgentPanel from './CreativeStudioAgentPanel';
import {
  CreativeStudioAgentChatController,
  CreativeStudioAgentProtocolError,
  type CreativeStudioAgentChatPort,
  type CreativeStudioAgentTurnEvent,
} from './chatPort';
import type { CreativeStudioAgentPanelProps } from './types';

const noop = () => undefined;
const model = {
  providerId: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001'),
  model: 'chat-model',
} as const;

const baseProps = (
  overrides: Partial<CreativeStudioAgentPanelProps> = {}
): CreativeStudioAgentPanelProps => ({
  view: 'chat',
  loadState: 'ready',
  sessions: [],
  activeSessionId: null,
  messages: [],
  draft: '',
  model,
  isRunning: false,
  onViewChange: noop,
  onNewSession: noop,
  onSelectSession: noop,
  onDraftChange: noop,
  onModelChange: noop,
  onSend: noop,
  onStop: noop,
  onCollapse: noop,
  ...overrides,
});

const renderPanel = (overrides: Partial<CreativeStudioAgentPanelProps> = {}) =>
  renderToStaticMarkup(<CreativeStudioAgentPanel {...baseProps(overrides)} />);

describe('CreativeStudioAgentPanel source-parity states', () => {
  test('renders the 390px right panel, source header, empty state and composer', () => {
    const html = renderPanel();

    expect(html.includes('data-creative-studio-agent-panel="true"')).toBe(true);
    expect(html.includes('创作 Agent')).toBe(true);
    expect(html.includes('查看历史记录')).toBe(true);
    expect(html.includes('新对话')).toBe(true);
    expect(html.includes('收起 Agent 面板')).toBe(true);
    expect(html.includes('data-agent-panel-state="empty"')).toBe(true);
    expect(html.includes('从一个想法开始')).toBe(true);
    expect(html.includes('描述创作目标，或让我继续操作画布')).toBe(true);
  });

  test('renders controlled history, loading and load-failure states', () => {
    const history = renderPanel({
      view: 'history',
      sessions: [{ id: 'session-1', title: '宣传片分镜', messageCount: 6, updatedAtLabel: '刚刚' }],
      activeSessionId: 'session-1',
    });
    const loading = renderPanel({ loadState: 'loading' });
    const failed = renderPanel({
      loadState: 'failed',
      errorMessage: '本地会话服务不可用',
      onRetryLoad: noop,
    });

    expect(history.includes('data-agent-panel-state="history"')).toBe(true);
    expect(history.includes('宣传片分镜')).toBe(true);
    expect(history.includes('6 条消息 · 刚刚')).toBe(true);
    expect(loading.includes('data-agent-panel-state="loading"')).toBe(true);
    expect(loading.includes('正在加载会话')).toBe(true);
    expect(failed.includes('data-agent-panel-state="failed"')).toBe(true);
    expect(failed.includes('本地会话服务不可用')).toBe(true);
  });

  test('renders supplied message, running, stopped and failed states without fake replies', () => {
    const html = renderPanel({
      messages: [
        { id: 'user-1', role: 'user', status: 'complete', text: '做一张海报' },
        {
          id: 'assistant-1',
          role: 'assistant',
          status: 'running',
          text: '',
          activityLabel: '正在分析当前画布',
        },
        {
          id: 'assistant-2',
          role: 'assistant',
          status: 'failed',
          text: '',
          errorMessage: '模型连接失败',
        },
        { id: 'assistant-3', role: 'assistant', status: 'stopped', text: '' },
      ],
      isRunning: true,
      onRetryMessage: noop,
    });

    expect(html.includes('data-agent-message-status="running"')).toBe(true);
    expect(html.includes('正在分析当前画布')).toBe(true);
    expect(html.includes('data-agent-message-status="failed"')).toBe(true);
    expect(html.includes('模型连接失败')).toBe(true);
    expect(html.includes('data-agent-message-status="stopped"')).toBe(true);
    expect(html.includes('已停止')).toBe(true);
    expect(html.includes('这是一个成功回复')).toBe(false);
  });
});

describe('Creative Studio Agent model and chat boundaries', () => {
  test('reuses the one NomiFun model catalog with the exact chat task', () => {
    const composer = readFileSync(new URL('./CreativeStudioAgentComposer.tsx', import.meta.url), 'utf8');

    expect(composer.includes('NomiCreativeModelSelect')).toBe(true);
    expect(composer.includes("capability: 'task'")).toBe(true);
    expect(composer.includes("task: 'chat'")).toBe(true);
    expect(composer.includes('useModelsForTask')).toBe(false);
    expect(composer.includes('useProvidersQuery')).toBe(false);
    expect(composer.includes('fetch(')).toBe(false);
  });

  test('requires an explicit completed event before reporting success', async () => {
    const seen: CreativeStudioAgentTurnEvent[] = [];
    const statuses: string[] = [];
    const port: CreativeStudioAgentChatPort = {
      async *runTurn() {
        yield { type: 'activity', label: '读取画布' };
        yield { type: 'assistant-delta', delta: '已完成' };
        yield { type: 'completed', assistantMessageId: 'assistant-1' };
      },
    };
    const controller = new CreativeStudioAgentChatController(port);

    const outcome = await controller.runTurn(
      {
        projectId: 'project-1',
        sessionId: 'session-1',
        prompt: '创建节点',
        model,
        history: [],
      },
      {
        onEvent: (event) => seen.push(event),
        onStatusChange: (status) => statuses.push(status.state),
      }
    );

    expect(outcome).toEqual({ state: 'completed' });
    expect(seen.map((event) => event.type)).toEqual([
      'activity',
      'assistant-delta',
      'completed',
    ]);
    expect(statuses).toEqual(['running', 'completed']);
    expect(controller.isRunning).toBe(false);
  });

  test('fails closed when an adapter stream ends without completion', async () => {
    const port: CreativeStudioAgentChatPort = {
      async *runTurn() {
        yield { type: 'assistant-delta', delta: 'partial' };
      },
    };
    const controller = new CreativeStudioAgentChatController(port);

    const outcome = await controller.runTurn({
      projectId: 'project-1',
      sessionId: 'session-1',
      prompt: '创建节点',
      model,
      history: [],
    });

    expect(outcome.state).toBe('failed');
    if (outcome.state === 'failed') {
      expect(outcome.error.name).toBe(CreativeStudioAgentProtocolError.name);
    }
  });

  test('stop aborts the injected adapter and never reports completion', async () => {
    let release: (() => void) | undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const port: CreativeStudioAgentChatPort = {
      async *runTurn(request) {
        await gate;
        if (request.signal.aborted) {
          const error = new Error('aborted');
          error.name = 'AbortError';
          throw error;
        }
        yield { type: 'completed' };
      },
    };
    const controller = new CreativeStudioAgentChatController(port);
    const turn = controller.runTurn({
      projectId: 'project-1',
      sessionId: 'session-1',
      prompt: '创建节点',
      model,
      history: [],
    });

    controller.stop();
    release?.();
    expect(await turn).toEqual({ state: 'stopped' });
    expect(controller.isRunning).toBe(false);
  });
});
