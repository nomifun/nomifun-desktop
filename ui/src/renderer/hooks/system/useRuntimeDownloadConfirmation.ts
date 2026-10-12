import { Modal } from '@arco-design/web-react';
import { useCallback, useLayoutEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

/** A confirmation belongs to its current page and never grants future downloads. */
export function useRuntimeDownloadConfirmation(owner?: unknown) {
  const { t } = useTranslation();
  const pending = useRef(new Set<() => void>());
  useLayoutEffect(() => () => {
    for (const cancel of [...pending.current]) cancel();
  }, [owner]);

  return useCallback((isCurrent: () => boolean): Promise<boolean> => {
    if (!isCurrent()) return Promise.resolve(false);
    return new Promise((resolve) => {
      let settled = false;
      let close = () => {};
      const settle = (accepted: boolean) => {
        if (settled) return;
        settled = true;
        pending.current.delete(cancel);
        resolve(accepted && isCurrent());
      };
      const cancel = () => { settle(false); close(); };
      const modal = Modal.confirm({
        title: t('common.runtimePreparation.downloadTitle', { tool: 'Bun' }),
        content: t('common.runtimePreparation.downloadHint', { tool: 'Bun' }),
        okText: t('common.runtimePreparation.downloadAction', { tool: 'Bun' }),
        cancelText: t('common.cancel'),
        onOk: () => settle(true),
        onCancel: () => settle(false),
        afterClose: () => settle(false),
      });
      close = () => modal.close();
      if (!settled) pending.current.add(cancel);
    });
  }, [t]);
}
