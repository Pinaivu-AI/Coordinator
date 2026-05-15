# Reproducible enclave image build for the Pinaivu coordinator.
#
# Skeleton — fleshed out alongside the init crate. Final target produces
# `coordinator.eif` and `coordinator.pcrs` via stagex multi-stage build.

# syntax=docker/dockerfile:1.7

FROM scratch AS base
# TODO(stagex): pull stagex core (musl libc, gcc, llvm, rust, binutils,
# ca-certs, openssl, git, busybox, socat, kernel headers).

FROM base AS build
# TODO: cargo build --release --features aws -p coordinator -p init -p aws -p system
# TODO: pack initramfs: coordinator binary, init binary, ca-certs,
#       busybox, socat, nsm.ko, run.sh
# TODO: invoke eif_build to emit coordinator.eif + coordinator.pcrs

FROM scratch AS install
# COPY --from=build /out/coordinator.eif   /coordinator.eif
# COPY --from=build /out/coordinator.pcrs  /coordinator.pcrs
