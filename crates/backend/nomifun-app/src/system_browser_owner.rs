//! Persistent ownership verification for the independent Agent browser port.
use nomifun_browser_platform::system_browser::SystemBrowserRuntimeError as Error;
use nomifun_db::IConversationRepository;
use std::sync::Arc;

pub(crate) struct SystemBrowserOwnerVerifier {
    pub owner: Arc<str>,
    pub conversations: Arc<dyn IConversationRepository>,
    pub execution: Arc<dyn nomifun_conversation::ExecutionConversationBoundary>,
}

#[async_trait::async_trait]
impl crate::system_browser::SystemBrowserOwnerVerifier for SystemBrowserOwnerVerifier {
    async fn verify(&self, user_id: &str, conversation_id: &str) -> Result<(), Error> {
        if user_id != self.owner.as_ref() {
            return Err(Error::TabDenied);
        }
        let row = self
            .conversations
            .get(conversation_id)
            .await
            .map_err(|_| Error::TabDenied)?
            .filter(|row| row.user_id == user_id)
            .ok_or(Error::TabDenied)?;
        if row.cron_job_id.is_some()
            || row
                .source
                .as_deref()
                .is_some_and(|source| source != "nomifun")
        {
            return Err(Error::TabDenied);
        }
        let projection = self
            .execution
            .projection(user_id, conversation_id)
            .await
            .map_err(|_| Error::TabDenied)?;
        if projection.execution_step_id.is_some() {
            return Err(Error::TabDenied);
        }
        Ok(())
    }
}
