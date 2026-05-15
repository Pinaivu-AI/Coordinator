.PHONY: check build test clean eif run-host

# Local dev — no enclave, mock NSM.
check:
	cargo check --workspace

build:
	cargo build --workspace

test:
	cargo test --workspace

# Reproducible Nitro Enclave image (Containerfile-driven).
# Produces coordinator.eif + coordinator.pcrs.
eif:
	@echo "TODO: wire stagex build via Containerfile"

# Run the parent-host socat forwarders.
run-host:
	./parent_forwarder.sh

clean:
	cargo clean
