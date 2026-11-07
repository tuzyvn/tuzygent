use agent_domain::TaskId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotentSubmission {
    Created(TaskId),
    Existing(TaskId),
    Conflict,
}
