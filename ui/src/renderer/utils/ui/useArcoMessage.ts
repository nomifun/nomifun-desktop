import { useRef } from 'react';
import { Message } from '@arco-design/web-react';

/**
 * arco-design (2.66.x) types the object returned by `Message.useMessage()` with
 * **optional** methods (`info?`, `success?`, `warning?`, `error?`, `loading?`,
 * `normal?`). Every `message.warning(...)` call site therefore trips TS2722
 * ("cannot invoke an object which is possibly 'undefined'"), even though the
 * hook always supplies the methods at runtime.
 *
 * This thin wrapper asserts the non-optional shape once, so call sites can use
 * `message.warning(...)` directly without a per-call cast or optional-chain.
 * Drop-in replacement for `Message.useMessage(...)`.
 *
 * It ALSO returns a **referentially stable** `message` instance. Arco recreates
 * its `message` api object on every render (no internal memoization, unlike
 * antd), so any component that lists `message` in a `useEffect`/`useCallback`
 * dependency array would re-run forever — an infinite render/fetch loop. We
 * therefore return a stable façade whose method calls always delegate to the
 * latest underlying api (tracked via a ref), making `[message]` deps safe.
 */
type UseMessageReturn = ReturnType<typeof Message.useMessage>;

export type ArcoMessageInstance = Required<UseMessageReturn[0]>;

export function useArcoMessage(
  config?: Parameters<typeof Message.useMessage>[0]
): [ArcoMessageInstance, UseMessageReturn[1]] {
  const [message, holder] = Message.useMessage(config);
  // Always point at the freshest api; the façade below reads through this ref.
  const latest = useRef(message);
  latest.current = message;
  // Build the stable façade exactly once and keep returning the same reference.
  const stable = useRef<ArcoMessageInstance | null>(null);
  if (stable.current === null) {
    stable.current = new Proxy({} as ArcoMessageInstance, {
      get(_target, prop, receiver) {
        const value = Reflect.get(latest.current as object, prop, receiver);
        return typeof value === 'function' ? value.bind(latest.current) : value;
      },
    });
  }
  return [stable.current, holder];
}
