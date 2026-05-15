//! Builds a `DispatchToken` for the winning bid, signs it with the
//! coordinator's enclave keypair, and sends it to the primary node
//! over the dispatch request-response protocol. Persists the matching
//! apalis job before returning so the timeout watchdog is armed.
