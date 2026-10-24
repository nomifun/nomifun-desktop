import '../../../../../test/setup-dom.ts';
import { cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import * as Context from './ExecutionContext';
import * as Layout from '../components/ChatLayout';
import * as Switcher from './ExecutionContentSwitcher';
import * as Panel from './ExecutionTopPanel';
import { parseConversationId, parseExecutionId } from '@/common/types/ids';
import type { ChatLayoutProps } from '../components/ChatLayout';
import ExecutionConversationLayout from './ExecutionConversationLayout';

let captured: Omit<ChatLayoutProps, 'children'> | undefined;
beforeEach(() => {
  captured = undefined;
  spyOn(Context, 'useExecution').mockReturnValue({ conversationId: parseConversationId('0190f5fe-7c00-7a00-8000-000000000511'),
    executionId: parseExecutionId('0190f5fe-7c00-7a00-8000-000000000512'), detail: null, refetch: async () => {},
    leadThinking: { phase: null, reasoning: '', active: false, textHeartbeat: false }, loading: false, projectedStepId: null,
    projectedPayload: null, projectStep: () => {}, projectStepAsync: async () => {}, returnToMain: () => {}, canvasOpen: true, setCanvasOpen: () => {}, toggleCanvas: () => {}, restartDrafts: new Map() });
  spyOn(Layout, 'default').mockImplementation(({ children, ...props }) => { captured = props; return <div>{children}</div>; });
  spyOn(Switcher, 'default').mockImplementation(({ children }) => <div data-testid='execution-switcher'>{children}</div>);
  spyOn(Panel, 'default').mockImplementation(() => <div data-testid='execution-top-panel' />);
});
afterEach(() => { cleanup(); mock.restore(); });

test('Apps keep ordinary conversation content visible even with a canonical execution link', () => {
  const view = render(<ExecutionConversationLayout embedded sider={null}><p>Ordinary app conversation</p></ExecutionConversationLayout>);
  expect(view.getByText('Ordinary app conversation')).toBeTruthy();
  expect(view.queryByTestId('execution-switcher')).toBeNull();
  expect(view.queryByTestId('execution-top-panel')).toBeNull();
  expect(captured?.workspaceCollaboration).toBeUndefined();
});

test('ordinary conversations retain their execution switcher, panel and collaboration controls', () => {
  const view = render(<ExecutionConversationLayout sider={null}><p>Ordinary conversation</p></ExecutionConversationLayout>);
  expect(view.getByTestId('execution-switcher')).toBeTruthy();
  expect(view.getByTestId('execution-top-panel')).toBeTruthy();
  expect(captured?.workspaceCollaboration?.available).toBe(true);
});
