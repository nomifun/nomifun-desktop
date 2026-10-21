import type { TChatConversation } from '@/common/config/storage';
import { useCallback, useMemo } from 'react';
import useSWR, { useSWRConfig } from 'swr';
import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';
import type { ConversationId } from '@/common/types/ids';
export type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';

type Draft = {
  _type: 'nomi';
  content: string;
  atPath: Array<string | FileOrFolderItem>;
  uploadFile: string[];
};

const store = new Map<ConversationId, Draft>();

/** Keep each conversation's composer draft across unmounts. */
export const getSendBoxDraftHook = (
  type: TChatConversation['type'],
  initialValue: Draft
) => {
  function useDraft(conversation_id: ConversationId) {
    const key = useMemo(() => [`/send-box/${type}/draft/${conversation_id}`, conversation_id] as const, [conversation_id]);
    const { data } = useSWR(
      key,
      ([_, id]) => store.get(id)
    );
    const { mutate } = useSWRConfig();
    const mutateDraft = useCallback(
      (update: (previous: Draft) => Draft | undefined): void => {
        // Bind the cache address to the original draft. SWR's bound mutate
        // follows the hook's latest key and is unsafe in a late old callback.
        void mutate<Draft | undefined>(
          key,
          (previous) => {
            const next = update(previous ?? initialValue);
            if (next) store.set(conversation_id, next);
            else store.delete(conversation_id);
            return next;
          },
          { revalidate: false }
        ).catch((error) => {
          console.error('Failed to mutate draft:', error);
        });
      },
      [conversation_id, key, mutate]
    );
    return { data, mutate: mutateDraft };
  }
  return useDraft;
};
