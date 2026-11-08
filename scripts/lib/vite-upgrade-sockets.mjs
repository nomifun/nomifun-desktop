// Vite leaves non-HMR upgrade requests unclaimed. Node hands those raw sockets
// to upgrade listeners without HTTP's error handler; a peer reset can otherwise
// terminate the entire development server. Keep this guard on this server only.
const installed = new WeakMap();

export function guardDevUpgradeSockets(server, logger) {
  if (!server) return () => {};
  const existing = installed.get(server);
  if (existing) return existing;
  const observed = new WeakSet();
  const onUpgrade = (_request, socket) => {
    if (observed.has(socket)) return;
    observed.add(socket);
    const onError = error => {
      // A WebSocket/proxy implementation that adopted the connection retains
      // its normal diagnostics and cleanup. Handle only otherwise-bare errors.
      if (socket.listeners('error').some(listener => listener !== onError)) return;
      socket.destroy();
      if (error?.code === 'ECONNRESET' || error?.code === 'EPIPE') {
        logger.info(`[dev] disconnected upgrade socket (${error.code})`, { timestamp: true });
      } else {
        const code = error?.code ? `[${error.code}] ` : '';
        const detail = error?.stack || error?.message || String(error);
        logger.error(`[dev] unclaimed upgrade socket failed: ${code}${detail}`, { timestamp: true, error });
      }
    };
    socket.on('error', onError);
  };
  const dispose = () => {
    server.off('upgrade', onUpgrade);
    server.off('close', dispose);
    installed.delete(server);
  };
  // Install before HMR/proxy dispatch, including a reset during handoff.
  server.prependListener('upgrade', onUpgrade);
  server.once('close', dispose);
  installed.set(server, dispose);
  return dispose;
}
