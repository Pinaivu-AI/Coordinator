//! Routing-receipt archive — where the coordinator stores the signed
//! audit artefacts it issues on completion. v1 ships an in-memory
//! impl; the Postgres-backed impl replaces it in slice 6 behind the
//! same trait.

use std::collections::HashMap;
use std::sync::RwLock;

use anyhow::Result;
use async_trait::async_trait;

use crate::protocol::{RequestId, RoutingReceipt};

#[async_trait]
pub trait ReceiptArchive: Send + Sync {
    async fn put(&self, receipt: RoutingReceipt) -> Result<()>;
    async fn get(&self, request_id: &RequestId) -> Result<Option<RoutingReceipt>>;
}

pub struct InMemoryReceiptArchive {
    receipts: RwLock<HashMap<RequestId, RoutingReceipt>>,
}

impl InMemoryReceiptArchive {
    pub fn new() -> Self {
        Self {
            receipts: RwLock::new(HashMap::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.receipts.read().unwrap().len()
    }
}

impl Default for InMemoryReceiptArchive {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ReceiptArchive for InMemoryReceiptArchive {
    async fn put(&self, receipt: RoutingReceipt) -> Result<()> {
        self.receipts
            .write()
            .unwrap()
            .insert(receipt.request_id, receipt);
        Ok(())
    }

    async fn get(&self, request_id: &RequestId) -> Result<Option<RoutingReceipt>> {
        Ok(self.receipts.read().unwrap().get(request_id).cloned())
    }
}
