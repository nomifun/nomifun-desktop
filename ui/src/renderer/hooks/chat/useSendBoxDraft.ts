import type { TChatConversation } from '@/common/config/storage';
import { useCallback } from 'react';
import useSWR from 'swr';
import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';
import type { ConversationId } from '@/common/types/ids';
export type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';

type Draft =
  | {
      _type: 'claude';
      content: unknown;
    }
  | {
      _type: 'nomi';
      content: string;
      atPath: Array<string | FileOrFolderItem>;
      uploadFile: string[];
    };

/**
 * 当前支持的对话类型以及对应的草稿对象
 */
type SendBoxDraftStore = {
  [K in TChatConversation['type']]: Map<ConversationId, Extract<Draft, { _type: K }>>;
};

const store: SendBoxDraftStore = {
  nomi: new Map(),
};

const setDraft = <K extends TChatConversation['type']>(
  type: K,
  conversation_id: ConversationId,
  draft: Extract<Draft, { _type: K }> | undefined
) => {
  // TODO import ts-pattern for exhaustive check
  switch (type) {
    case 'nomi':
      if (draft) {
        store.nomi.set(conversation_id, draft as Extract<Draft, { _type: 'nomi' }>);
      } else {
        store.nomi.delete(conversation_id);
      }
      break;
    default:
      break;
  }
};

const getDraft = <K extends TChatConversation['type']>(
  type: K,
  conversation_id: ConversationId
): Extract<Draft, { _type: K }> | undefined => {
  // TODO import ts-pattern for exhaustive check
  switch (type) {
    case 'nomi':
      return store.nomi.get(conversation_id) as Extract<Draft, { _type: K }>;
    default:
      return undefined;
  }
};

/**
 * 获得一种类型下的会话草稿操作的 React Hook
 */
export const getSendBoxDraftHook = <K extends TChatConversation['type']>(
  type: K,
  initialValue: Extract<Draft, { _type: K }>
) => {
  function useDraft(conversation_id: ConversationId) {
    const swrRet = useSWR([`/send-box/${type}/draft/${conversation_id}`, conversation_id], ([_, id]) => {
      return getDraft(type, id);
    });

    const mutateDraft = useCallback(
      (draft: (k: Extract<Draft, { _type: K }>) => typeof k | undefined): void => {
        swrRet
          .mutate(
            (prev) => {
              const newDraft = draft(prev ?? initialValue);
              setDraft(type, conversation_id, newDraft);
              return newDraft;
            },
            { revalidate: false }
          )
          .catch((error) => {
            console.error('Failed to mutate draft:', error);
          });
      },
      [conversation_id]
    );

    return {
      get data() {
        return swrRet.data;
      },
      mutate: mutateDraft,
    };
  }

  return useDraft;
};
