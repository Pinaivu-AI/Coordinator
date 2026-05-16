REGISTRY := local
.DEFAULT_GOAL := eif

.PHONY: check build test clean eif run run-debug run-local stop logs status

out:
	mkdir -p out

# ── Primary target: build the EIF and PCRs ───────────────────────────────────
eif: out
	docker build \
		--tag $(REGISTRY)/pinaivu-coordinator \
		--progress=plain \
		--platform linux/amd64 \
		--output type=local,rewrite-timestamp=true,dest=out \
		-f Containerfile \
		.

# ── Local dev ─────────────────────────────────────────────────────────────────
check:
	cargo check --workspace

build:
	cargo build --workspace

test:
	cargo test --workspace

# ── Enclave management (EC2 only) ─────────────────────────────────────────────
run: out/coordinator.eif
	sudo nitro-cli \
		run-enclave \
		--cpu-count 2 \
		--memory 4096 \
		--eif-path out/coordinator.eif
	@echo ""
	@echo "Enclave running. Start host bridges:"
	@echo "  ./parent_forwarder.sh"
	@echo ""
	@echo "Smoke test:"
	@echo "  curl http://localhost:4000/health"
	@echo "  curl http://localhost:4000/enclave_health"

run-debug: out/coordinator.eif
	sudo nitro-cli \
		run-enclave \
		--cpu-count 2 \
		--memory 4096 \
		--eif-path out/coordinator.eif \
		--debug-mode \
		--attach-console

run-local:
	cargo run -p coordinator

stop:
	sudo nitro-cli terminate-enclave --all

logs:
	sudo nitro-cli console --enclave-name \
		$$(sudo nitro-cli describe-enclaves | jq -r '.[0].EnclaveID')

status:
	@echo "=== ENCLAVE STATUS ==="
	sudo nitro-cli describe-enclaves 2>/dev/null || echo "No enclaves running"

# ── Host bridges ──────────────────────────────────────────────────────────────
run-host:
	./parent_forwarder.sh

clean:
	cargo clean
	rm -rf out
