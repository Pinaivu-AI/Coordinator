//! `InMemoryMesh` — a [`Mesh`](super::Mesh) impl used in tests. Lets
//! a test pre-seed a list of bids; when the coordinator publishes a
//! request, every seeded bid is rewritten with the live `request_id`
//! and immediately fed into the auction's receiver.

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{InferenceDispatch, InferenceReply, Mesh};
use crate::protocol::{InferenceBid, InferenceRequest};

pub struct InMemoryMesh {
    seeded: Mutex<Vec<InferenceBid>>,
}

impl InMemoryMesh {
    pub fn new() -> Self {
        Self {
            seeded: Mutex::new(Vec::new()),
        }
    }

    /// Pre-seed the bids that will be delivered next time the auction
    /// publishes a request. The seeded bids' `request_id` is rewritten
    /// at publish time so a single seed plays for any request.
    pub fn seed_bids(&self, bids: Vec<InferenceBid>) {
        let mut g = self.seeded.lock().unwrap();
        *g = bids;
    }
}

impl Default for InMemoryMesh {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mesh for InMemoryMesh {
    async fn publish_request(
        &self,
        request: &InferenceRequest,
    ) -> Result<mpsc::Receiver<InferenceBid>> {
        let bids = {
            let g = self.seeded.lock().unwrap();
            g.clone()
        };
        let (tx, rx) = mpsc::channel(64.max(bids.len()));
        for mut bid in bids {
            bid.request_id = request.request_id;
            // ignore send errors — test will fail downstream if rx
            // was already dropped
            let _ = tx.send(bid).await;
        }
        Ok(rx)
    }

    /// No real node to dial — return a canned reply naming the peer
    /// it was asked to dispatch to, so tests can assert on it.
    async fn dispatch_inference(
        &self,
        peer: libp2p::PeerId,
        dispatch: InferenceDispatch,
    ) -> Result<InferenceReply> {
        Ok(InferenceReply {
            request_id: dispatch.dispatch_token.request_id,
            session_id: dispatch.dispatch_token.session_id,
            content: format!("mock-reply-from-{peer}"),
            input_tokens: 10,
            output_tokens: 20,
            latency_ms: 5,
            error: None,
        })
    }
}
