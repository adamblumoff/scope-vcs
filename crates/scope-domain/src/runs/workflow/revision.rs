use super::{definition::CompiledWorkflow, error::WorkflowError, identity::WorkflowIdentity};
use serde::Serialize;
use sha2::{Digest, Sha256};

const WORKFLOW_DIGEST_VERSION: u8 = 6;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowRevision {
    workflow: WorkflowIdentity,
    digest: String,
    definition: CompiledWorkflow,
}

impl WorkflowRevision {
    pub fn new(
        workflow: WorkflowIdentity,
        definition: CompiledWorkflow,
    ) -> Result<Self, WorkflowError> {
        #[derive(Serialize)]
        struct DigestInput<'a> {
            version: u8,
            definition: &'a CompiledWorkflow,
        }

        let bytes = serde_json::to_vec(&DigestInput {
            version: WORKFLOW_DIGEST_VERSION,
            definition: &definition,
        })
        .map_err(WorkflowError::Digest)?;
        let digest = hex::encode(Sha256::digest(bytes));
        Ok(Self {
            workflow,
            digest,
            definition,
        })
    }

    pub fn workflow(&self) -> &WorkflowIdentity {
        &self.workflow
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn definition(&self) -> &CompiledWorkflow {
        &self.definition
    }
}
