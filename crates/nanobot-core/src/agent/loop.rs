use anyhow::Result;

use crate::bus::InboundMessage;
use crate::bus::OutboundMessage;
use crate::session::SessionManager;

/// Core agent processing loop.
pub struct AgentLoop {
    pub sessions: SessionManager,
}

impl AgentLoop {
    pub fn new(sessions: SessionManager) -> Self {
        Self { sessions }
    }

    /// Process a single inbound message and produce a response.
    pub async fn process_message(&mut self, _msg: &InboundMessage) -> Result<OutboundMessage> {
        // TODO: build context, call LLM via rig, execute tools, return response
        todo!("agent loop not yet implemented")
    }
}
