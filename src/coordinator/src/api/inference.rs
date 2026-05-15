//! `POST /v1/chat/completions` — OpenAI-shaped entry point.
//!
//! Runs the auction, signs a `DispatchToken`, persists the apalis job,
//! and returns `{ node_url, dispatch_token }` to the client. The
//! client then opens its own HTTPS connection to `node_url` to receive
//! the streamed response.
