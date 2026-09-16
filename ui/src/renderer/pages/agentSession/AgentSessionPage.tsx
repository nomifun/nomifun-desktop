import { parseConversationId } from '@/common/types/ids';
import { Navigate, useParams } from 'react-router-dom';

/** Historical Agent Session links use the canonical Conversation identity. */
export default function AgentSessionPage() {
  const { agentSessionId } = useParams();
  try {
    const conversationId = parseConversationId(agentSessionId);
    return <Navigate replace to={`/conversation/${encodeURIComponent(conversationId)}`} />;
  } catch {
    return <Navigate replace to='/agent' />;
  }
}
