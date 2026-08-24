use auditeur::prelude::anyhow;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Module(#[from] crate::lazy::ModuleError),
    #[error("custom: {0}")]
    Custom(#[from] anyhow::Error),
}
