use nomi_protocol::commands::ApprovalScope;
use nomi_protocol::events::ToolCategory;
use nomi_protocol::{ToolApprovalManager, ToolApprovalResult};
use rstest::rstest;

#[rstest]
#[case(ApprovalScope::Once, ToolCategory::Exec, "exec", false)]
#[case(ApprovalScope::Always, ToolCategory::Edit, "edit", true)]
#[tokio::test]
async fn approve_resolves_request_and_updates_auto_approval(
    #[case] scope: ApprovalScope,
    #[case] category: ToolCategory,
    #[case] category_name: &str,
    #[case] should_auto_approve: bool,
) {
    let manager = ToolApprovalManager::new();
    let rx = manager.request_approval("call-1", &category);

    manager.approve("call-1", scope);

    let result = rx.await.expect("approval result should arrive");
    assert!(matches!(result, ToolApprovalResult::Approved));
    assert_eq!(manager.is_auto_approved(category_name), should_auto_approve);
}

#[tokio::test]
async fn resolve_preserves_denial_reason() {
    let manager = ToolApprovalManager::new();
    let rx = manager.request_approval("call-2", &ToolCategory::Exec);

    manager.resolve(
        "call-2",
        ToolApprovalResult::Denied {
            reason: "policy violation".to_string(),
        },
    );

    let result = rx.await.expect("denial result should arrive");
    assert!(matches!(
        result,
        ToolApprovalResult::Denied { reason } if reason == "policy violation"
    ));
    assert!(!manager.is_auto_approved("exec"));
}
