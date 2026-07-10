import type { AssistantTagPickerHandle } from './AssistantTagPicker';

type PendingTagPickerRef = {
  current: Pick<AssistantTagPickerHandle, 'resetPendingTag'> | null;
};

/** Coordinates unfinished tag drafts at the assistant drawer boundary. */
export const createAssistantTagDraftLifecycle = (
  audiencePickerRef: PendingTagPickerRef,
  scenarioPickerRef: PendingTagPickerRef,
  setEditVisible: (visible: boolean) => void,
  handleSave: () => void
) => {
  const resetPendingTagDrafts = () => {
    audiencePickerRef.current?.resetPendingTag();
    scenarioPickerRef.current?.resetPendingTag();
  };

  return {
    resetPendingTagDrafts,
    closeDrawer: () => {
      resetPendingTagDrafts();
      setEditVisible(false);
    },
    handleDrawerSave: () => {
      resetPendingTagDrafts();
      handleSave();
    },
  };
};
