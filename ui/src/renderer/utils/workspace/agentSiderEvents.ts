export const AGENT_SIDER_TOGGLE_EVENT = 'nomifun-agent-sider-toggle';
export const AGENT_SIDER_STATE_EVENT = 'nomifun-agent-sider-state';
export const dispatchAgentSiderToggleEvent = () => window.dispatchEvent(new CustomEvent(AGENT_SIDER_TOGGLE_EVENT));
export const dispatchAgentSiderStateEvent = (collapsed: boolean) => window.dispatchEvent(new CustomEvent(AGENT_SIDER_STATE_EVENT, { detail: { collapsed } }));
