//! Pending tool-call approvals and per-chat grants.

use std::collections::{HashMap, HashSet};

use novumchat_core::events::AppEvent;
use novumchat_core::types::{ApprovalDecision, ApprovalRequest, Id};
use parking_lot::Mutex;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::events::EventHub;

#[derive(Default)]
pub struct Approvals {
    pending: Mutex<HashMap<Id, (ApprovalRequest, oneshot::Sender<ApprovalDecision>)>>,
    grants: Mutex<HashMap<Id, HashSet<String>>>,
}

impl Approvals {
    /// Asks the user and waits. Cancellation counts as a denial.
    pub async fn request(
        &self,
        events: &EventHub,
        req: ApprovalRequest,
        cancel: &CancellationToken,
    ) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
        let id = req.id.clone();
        self.pending.lock().insert(id.clone(), (req.clone(), tx));
        events.emit(AppEvent::ApprovalRequested { request: req.clone() });
        let decision = tokio::select! {
            d = rx => d.unwrap_or(ApprovalDecision::Deny),
            _ = cancel.cancelled() => ApprovalDecision::Deny,
        };
        self.pending.lock().remove(&id);
        if decision == ApprovalDecision::AllowForChat {
            self.grant(&req.chat_id, &req.tool);
        }
        events.emit(AppEvent::ApprovalResolved { id, decision });
        decision
    }

    pub fn resolve(&self, id: &str, decision: ApprovalDecision) -> bool {
        match self.pending.lock().remove(id) {
            Some((_, tx)) => tx.send(decision).is_ok(),
            None => false,
        }
    }

    pub fn pending(&self) -> Vec<ApprovalRequest> {
        let mut v: Vec<ApprovalRequest> = self.pending.lock().values().map(|(r, _)| r.clone()).collect();
        v.sort_by_key(|r| r.created_at);
        v
    }

    pub fn grant(&self, chat_id: &str, tool: &str) {
        self.grants.lock().entry(chat_id.to_string()).or_default().insert(tool.to_string());
    }

    pub fn is_granted(&self, chat_id: &str, tool: &str) -> bool {
        self.grants.lock().get(chat_id).is_some_and(|s| s.contains(tool))
    }

    pub fn forget_chat(&self, chat_id: &str) {
        self.grants.lock().remove(chat_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use novumchat_core::permissions::ToolRisk;
    use std::sync::Arc;

    fn req(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            id: id.into(),
            chat_id: "c".into(),
            message_id: "m".into(),
            tool_call_id: "t".into(),
            tool: "bash".into(),
            title: "Run".into(),
            detail: "ls".into(),
            risk: ToolRisk::Execute,
            created_at: 0,
        }
    }

    #[tokio::test]
    async fn resolve_and_grant() {
        let approvals = Arc::new(Approvals::default());
        let hub = Arc::new(EventHub::new());
        let a = approvals.clone();
        let h = hub.clone();
        let task = tokio::spawn(async move { a.request(&h, req("1"), &CancellationToken::new()).await });
        while approvals.pending().is_empty() {
            tokio::task::yield_now().await;
        }
        assert!(approvals.resolve("1", ApprovalDecision::AllowForChat));
        assert_eq!(task.await.unwrap(), ApprovalDecision::AllowForChat);
        assert!(approvals.is_granted("c", "bash"));
        assert!(!approvals.is_granted("c", "write_file"));

        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(approvals.request(&hub, req("2"), &cancel).await, ApprovalDecision::Deny);
        assert!(approvals.pending().is_empty());
    }
}
