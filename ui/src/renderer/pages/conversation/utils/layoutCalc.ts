// Layout constants for the chat layout panel sizing
export const MIN_CHAT_RATIO = 25;
export const MIN_PREVIEW_RATIO = 20;
export const WORKSPACE_HEADER_HEIGHT = 32;
export const MIN_CHAT_PANEL_PX = 360;
export const MIN_PREVIEW_PANEL_PX = 340;
export const MIN_WORKSPACE_PANEL_PX = 220;
export const MAX_WORKSPACE_PANEL_PX = 500;
export const DEFAULT_WORKSPACE_PANEL_PX = 260;

export type LayoutCalcInput = {
  containerWidth: number;
  /** 用户偏好的工作空间宽度（像素） */
  workspaceWidthPx: number;
  chatSplitRatio: number;
  workspaceEnabled: boolean;
  isPreviewOpen: boolean;
  rightSiderCollapsed: boolean;
};

export type LayoutMetrics = {
  /** 工作空间生效宽度（像素，已根据容器宽度做上限收缩） */
  effectiveWorkspaceWidthPx: number;
  dynamicChatMinRatio: number;
  dynamicChatMaxRatio: number;
  /** 聊天区在主区域内的 flex-grow（preview 关闭时为 100，打开时按 chatSplitRatio 分配） */
  chatFlex: number;
  /** 工作空间最终展示像素 */
  workspaceWidthPx: number;
  titleAreaMaxWidth: number;
};

/**
 * Compute all derived layout metrics. Workspace width is fixed px, not a
 * percentage of the container — the chat area absorbs viewport size changes.
 */
export const calcLayoutMetrics = (input: LayoutCalcInput): LayoutMetrics => {
  const {
    containerWidth,
    workspaceWidthPx: requestedWorkspaceWidthPx,
    chatSplitRatio,
    workspaceEnabled,
    isPreviewOpen,
    rightSiderCollapsed,
  } = input;

  const safeContainerWidth = Math.max(containerWidth || 0, 1);

  // Workspace uses a fixed pixel width and shrinks to the container limit.
  const workspaceVisible = workspaceEnabled && !rightSiderCollapsed;
  const previewReservedPx = isPreviewOpen ? MIN_CHAT_PANEL_PX + MIN_PREVIEW_PANEL_PX : MIN_CHAT_PANEL_PX;
  const workspaceMaxByContainer = Math.max(MIN_WORKSPACE_PANEL_PX, safeContainerWidth - previewReservedPx);
  const effectiveWorkspaceWidthPx = workspaceVisible
    ? Math.max(
        MIN_WORKSPACE_PANEL_PX,
        Math.min(MAX_WORKSPACE_PANEL_PX, requestedWorkspaceWidthPx, workspaceMaxByContainer)
      )
    : 0;

  // 计算 chat / preview 之间的动态比例约束（基于剩余可用宽度）
  const availableWidthForChatPreview = Math.max(safeContainerWidth - effectiveWorkspaceWidthPx, 1);
  const minChatRatioByPx = (MIN_CHAT_PANEL_PX / availableWidthForChatPreview) * 100;
  const minPreviewRatioByPx = (MIN_PREVIEW_PANEL_PX / availableWidthForChatPreview) * 100;
  const dynamicChatMinRatio =
    workspaceEnabled && isPreviewOpen ? Math.max(MIN_CHAT_RATIO, minChatRatioByPx) : MIN_CHAT_RATIO;
  const dynamicChatMaxCandidate =
    workspaceEnabled && isPreviewOpen
      ? Math.min(80, 100 - Math.max(MIN_PREVIEW_RATIO, minPreviewRatioByPx))
      : 80;
  const dynamicChatMaxRatio = Math.max(dynamicChatMinRatio, dynamicChatMaxCandidate);

  // chat-area flex（外层 chat+preview 容器内的聊天面板的 flex-grow）：
  // preview 打开时按 chatSplitRatio 分配，否则聊天区独占
  const chatFlex = isPreviewOpen ? chatSplitRatio : 100;
  const workspaceWidthPx = workspaceEnabled ? effectiveWorkspaceWidthPx : 0;
  const titleAreaMaxWidth = Math.max(320, Math.min(820, containerWidth - 520));

  return {
    effectiveWorkspaceWidthPx,
    dynamicChatMinRatio,
    dynamicChatMaxRatio,
    chatFlex,
    workspaceWidthPx,
    titleAreaMaxWidth,
  };
};
