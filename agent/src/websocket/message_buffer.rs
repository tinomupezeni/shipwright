/// Message buffer for replaying recent deployment events to new WebSocket clients
use shipwright_common::protocol::AgentMessage;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const MAX_BUFFER_SIZE: usize = 200; // Keep last 200 messages

#[derive(Clone)]
pub struct MessageBuffer {
    buffer: Arc<Mutex<VecDeque<AgentMessage>>>,
}

impl MessageBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_BUFFER_SIZE))),
        }
    }

    /// Add a message to the buffer (called when broadcasting)
    pub fn push(&self, message: AgentMessage) {
        let mut buffer = self.buffer.lock().unwrap();

        // Only buffer deployment-related messages, skip metrics
        match message {
            AgentMessage::BuildUpdate { .. } |
            AgentMessage::RollbackUpdate { .. } |
            AgentMessage::Error(_) => {
                buffer.push_back(message);

                // Keep buffer size limited
                if buffer.len() > MAX_BUFFER_SIZE {
                    buffer.pop_front();
                }
            }
            _ => {} // Don't buffer metrics or other transient messages
        }
    }

    /// Get all buffered messages for replay
    pub fn get_all(&self) -> Vec<AgentMessage> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter().cloned().collect()
    }

    /// Get messages for a specific project
    pub fn get_for_project(&self, project_name: &str) -> Vec<AgentMessage> {
        let buffer = self.buffer.lock().unwrap();
        buffer.iter()
            .filter(|msg| {
                match msg {
                    AgentMessage::BuildUpdate { project_name: pn, .. } => pn == project_name,
                    AgentMessage::RollbackUpdate { project_name: pn, .. } => pn == project_name,
                    _ => false,
                }
            })
            .cloned()
            .collect()
    }

    /// Clear old messages (could be called periodically)
    pub fn clear(&self) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shipwright_common::protocol::BuildEvent;

    #[test]
    fn test_message_buffer() {
        let buffer = MessageBuffer::new();

        // Add some messages
        buffer.push(AgentMessage::BuildUpdate {
            project_name: "test-project".to_string(),
            event: BuildEvent::Started,
        });

        buffer.push(AgentMessage::BuildUpdate {
            project_name: "test-project".to_string(),
            event: BuildEvent::Log("Building...".to_string()),
        });

        // Get all messages
        let messages = buffer.get_all();
        assert_eq!(messages.len(), 2);

        // Get project-specific messages
        let project_messages = buffer.get_for_project("test-project");
        assert_eq!(project_messages.len(), 2);

        let other_messages = buffer.get_for_project("other-project");
        assert_eq!(other_messages.len(), 0);
    }
}
