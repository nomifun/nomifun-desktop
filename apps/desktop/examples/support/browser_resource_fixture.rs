use nomifun_browser_platform::{
    product::{
        BrowserCapabilityAction, BrowserProviderDescriptor, BrowserResourceBinding,
        BrowserSessionAuthority,
    },
    runtime::BrowserResourceKey,
};

pub(crate) fn authority(
    principal_id: &str,
    agent_session_id: &str,
    provider_identity: &str,
) -> BrowserSessionAuthority {
    BrowserSessionAuthority::new(
        principal_id,
        agent_session_id,
        BrowserCapabilityAction::all(),
        BrowserResourceBinding::new(
            format!("browser-fixture:{agent_session_id}"),
            "managed-browser",
            principal_id,
            BrowserProviderDescriptor::managed("managed", provider_identity)
                .expect("fixture provider identity"),
            BrowserCapabilityAction::all()
                .map(BrowserCapabilityAction::resource_operation),
        )
        .expect("fixture Browser Resource binding"),
    )
    .expect("fixture Browser AgentSession authority")
}

pub(crate) fn key(principal_id: &str, agent_session_id: &str) -> BrowserResourceKey {
    authority(principal_id, agent_session_id, "native-fixture").key()
}
