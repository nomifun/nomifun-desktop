import type { IMessageText, TMessage } from '@/common/chat/chatLib';
import { normalizeToolMessages, type ToolMessage } from '@/common/chat/normalizeToolCall';

export interface CompletionOutcomeDisplay {
  body: string;
  toolCount: number;
  commandCount: number;
  /** Only exact same-turn native outcomes can establish this presentation. */
  kind: 'native_nonzero' | 'native_nonzero_and_arguments' | 'native_nonzero_and_unclassified' | 'arguments_not_executed' | 'counts';
  exitCodes: number[];
  argumentCount?: number;
  otherCount?: number;
}

const toolFooter = String.raw`Unsuccessful tool attempts in this turn: ([1-9]\d{0,9}) \(including argument checks and command outcomes\)\. Details remain available in the execution steps\.`;
const commandFooter = String.raw`Unsuccessful command attempts in this turn: ([1-9]\d{0,9})\. Each command's exit status and output explain the result\.`;
const suffix = new RegExp(`\\n\\n(?:${toolFooter}(?:\\n\\n${commandFooter})?|${commandFooter})$`);

const hasOpenFence = (text: string): boolean => {
  let fence: { marker: string; length: number } | undefined;
  for (const line of text.split('\n')) {
    const match = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (!match) continue;
    const marker = match[1][0];
    if (!fence) fence = { marker, length: match[1].length };
    else if (fence.marker === marker && match[1].length >= fence.length) fence = undefined;
  }
  return fence !== undefined;
};

/** Project the exact runtime footer for display; canonical text stays intact. */
export function projectCompletionOutcomes(
  message: IMessageText,
  text: string,
  messages: readonly TMessage[]
): CompletionOutcomeDisplay | undefined {
  if (message.position !== 'left' || message.content.agentMessage === true) return undefined;
  const match = suffix.exec(text);
  if (!match || hasOpenFence(text.slice(0, match.index))) return undefined;
  const toolCount = Number(match[1] ?? 0);
  const commandCount = Number(match[2] ?? match[3] ?? 0);
  if (toolCount > 0xffffffff || commandCount > 0xffffffff) return undefined;
  const body = text.slice(0, match.index);
  const precedingParagraph = body.slice(body.lastIndexOf('\n\n') + 2);
  if (precedingParagraph.startsWith('Unsuccessful tool attempts in this turn:')
    || precedingParagraph.startsWith('Unsuccessful command attempts in this turn:')) return undefined;
  const sameTurn = message.turn_id
    ? messages.filter((item): item is ToolMessage => item.conversation_id === message.conversation_id
      && item.turn_id === message.turn_id && (item.type === 'tool_call' || item.type === 'tool_group'))
    : [];
  const tools = [...new Map(normalizeToolMessages(sameTurn).map((tool) => [tool.key, tool])).values()];
  const ordinaryExits = tools.filter((tool) => tool.nonFatalFailure === true
    && tool.commandExitCode !== undefined && tool.commandExitCode > 0);
  const ordinaryKeys = new Set(ordinaryExits.map((tool) => tool.key));
  const rejectedArguments = tools.filter((tool) => tool.notExecutedReason === 'invalid_arguments');
  const argumentKeys = new Set(rejectedArguments.map((tool) => tool.key));
  const otherFailures = tools.some((tool) => !ordinaryKeys.has(tool.key) && !argumentKeys.has(tool.key)
    && (tool.status === 'error' || tool.notExecutedReason !== undefined
      || tool.boundedResult !== undefined || tool.nonFatalFailure === true));
  // Classify only a fully reconciled breakdown from the same turn. A missing
  // receipt or another failure must keep the generic counts and full details.
  const breakdownMatches = commandCount === ordinaryExits.length
    && toolCount === ordinaryExits.length + rejectedArguments.length && !otherFailures;
  if (rejectedArguments.length > 0 && breakdownMatches) {
    return { body, toolCount, commandCount,
      kind: ordinaryExits.length > 0 ? 'native_nonzero_and_arguments' : 'arguments_not_executed',
      exitCodes: [...new Set(ordinaryExits.map((tool) => tool.commandExitCode!))],
      argumentCount: rejectedArguments.length };
  }
  if (ordinaryExits.length > 0 && commandCount === ordinaryExits.length
    && toolCount > ordinaryExits.length + rejectedArguments.length && !otherFailures) {
    // History may omit internal controls. Identify only the proven command
    // outcomes; the remaining total does not prove an argument or host fault.
    return { body, toolCount, commandCount, kind: 'native_nonzero_and_unclassified',
      exitCodes: [...new Set(ordinaryExits.map((tool) => tool.commandExitCode!))],
      otherCount: toolCount - ordinaryExits.length };
  }
  const nativeOnly = ordinaryExits.length > 0 && commandCount === ordinaryExits.length
    && (toolCount === 0 || toolCount === ordinaryExits.length) && rejectedArguments.length === 0 && !otherFailures;
  return { body, toolCount, commandCount, kind: nativeOnly ? 'native_nonzero' : 'counts',
    exitCodes: nativeOnly ? [...new Set(ordinaryExits.map((tool) => tool.commandExitCode!))] : [] };
}
