import type { TChatConversation } from '@/common/config/storage';
import { useCallback } from 'react';
import useSWR from 'swr';
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
    const { data, mutate } = useSWR(
      [`/send-box/${type}/draft/${conversation_id}`, conversation_id],
      ([_, id]) => store.get(id)
    );
    const mutateDraft = useCallback(
      (update: (previous: Draft) => Draft | undefined): void => {
        void mutate(
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
      [conversation_id, mutate]
    );
    return { data, mutate: mutateDraft };
  }
  return useDraft;
};
