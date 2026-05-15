//! Gossipsub topic names.

pub const INFERENCE_ANY: &str = "/pinaivu/inference/any/1.0.0";
pub const BIDS: &str = "/pinaivu/bids/1.0.0";
pub const ANNOUNCE: &str = "/pinaivu/announce/1.0.0";
pub const REPUTATION: &str = "/pinaivu/reputation/1.0.0";

pub fn inference_for_model(model_id: &str) -> String {
    format!("/pinaivu/inference/{model_id}/1.0.0")
}
