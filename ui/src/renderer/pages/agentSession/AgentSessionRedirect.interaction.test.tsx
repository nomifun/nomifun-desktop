import '../../../../test/setup-dom.ts';
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { MemoryRouter, Route, Routes, useParams } from 'react-router-dom';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import AgentSessionPage from './AgentSessionPage';

afterEach(() => { cleanup(); mock.restore(); });
test('historical Agent Session links open the existing canonical conversation without creating a session or plugin surface', async () => {
  const id = '0190f5fe-7c00-7a00-8000-000000000101';
  const create = spyOn(agentPlatform.sessions.create, 'invoke');
  const turn = spyOn(agentPlatform.sessions.createTurn, 'invoke');
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke');
  const StandardConversation = () => <div>Standard conversation {useParams().conversationId}</div>;
  const view = render(<MemoryRouter initialEntries={[`/agent-sessions/${id}`]}><Routes>
    <Route path='/agent-sessions/:agentSessionId' element={<AgentSessionPage />} />
    <Route path='/conversation/:conversationId' element={<StandardConversation />} />
  </Routes></MemoryRouter>);
  await view.findByText(`Standard conversation ${id}`);
  expect(create).not.toHaveBeenCalled(); expect(turn).not.toHaveBeenCalled(); expect(open).not.toHaveBeenCalled();
});
test('invalid historical identity returns to Agent management without inventing a conversation', async () => {
  const view = render(<MemoryRouter initialEntries={['/agent-sessions/not-an-id']}><Routes>
    <Route path='/agent-sessions/:agentSessionId' element={<AgentSessionPage />} />
    <Route path='/agent' element={<div>Agent management</div>} />
  </Routes></MemoryRouter>);
  await waitFor(() => expect(view.getByText('Agent management')).toBeTruthy());
});
