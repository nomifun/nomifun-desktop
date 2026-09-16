//! Context-scoped removal. A hidden maintenance child owns the protocol only
//! after every actual tab in this conversation has confirmed destruction.
use super::*;
use tokio_util::sync::CancellationToken;

impl Page {
    pub async fn clear_site_data(self: &Arc<Self>, cancel: CancellationToken) -> Result<(), String> {
        let page = self.clone();
        let allowed: Arc<crate::protocol::DispatchGuard> = Arc::new(move || {
            !cancel.is_cancelled() && !page.visible.load(Ordering::Acquire)
                && !page.close_requested.load(Ordering::Acquire)
                && page.snapshot().url == BOOTSTRAP_URL
                && !page.engine.pages.lock().unwrap().values().filter_map(Weak::upgrade)
                    .any(|other| other.id != page.id && Arc::ptr_eq(&other._context, &page._context))
        });
        // The pinned Chromium Storage handler maps an opaque wildcard origin to
        // all storage keys in *this request context's storage partition*.
        // Native A/B fixtures verify this scope, including IndexedDB and cookies.
        self.protocol.call_guarded(None, "Storage.clearDataForOrigin", serde_json::json!({"origin":"*","storageTypes":"all"}), Some(allowed.clone())).await?;
        self.protocol.call_guarded(None, "Network.clearBrowserCache", serde_json::json!({}), Some(allowed.clone())).await?;
        for kind in [0, 1, 2] {
            let page = self.clone();
            let allowed = allowed.clone();
            let (tx, rx) = oneshot::channel();
            self.engine.post(Box::new(move || {
                if !allowed() { let _ = tx.send(Err("CEF site-data authority changed".to_owned())); return; }
                let context = page._context.raw.lock().unwrap().clone();
                let Some(context) = context else { let _ = tx.send(Err("CEF request context is closed".to_owned())); return; };
                let mut callback = Cleared::new(Arc::new(Mutex::new(Some(tx))));
                match kind {
                    0 => context.clear_http_auth_credentials(Some(&mut callback)),
                    1 => context.clear_certificate_exceptions(Some(&mut callback)),
                    _ => context.close_all_connections(Some(&mut callback)),
                }
            }))?;
            rx.await.map_err(|_| "CEF site-data completion acknowledgement was lost")??;
        }
        Ok(())
    }
}

wrap_completion_callback! { struct Cleared { sender: Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>, } impl CompletionCallback {
    fn on_complete(&self) {
        if let Some(sender) = self.sender.lock().unwrap().take() { let _ = sender.send(Ok(())); }
    }
} }
