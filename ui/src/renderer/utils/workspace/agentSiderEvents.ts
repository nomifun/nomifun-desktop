import { createContentSiderChannel } from '@/renderer/components/layout/ContentSider/createContentSiderChannel';

export const agentSiderChannel = createContentSiderChannel('agent');
export const AGENT_SIDER_TOGGLE_EVENT = agentSiderChannel.toggleEvent;
export const dispatchAgentSiderStateEvent = agentSiderChannel.dispatchState;
