// Convert a hex string of NSM attestation bytes into a Sui CLI PTB
// vector literal: `[N1u8,N2u8,...]`. Used by register-coordinator.sh
// so the deploy step can build the register_enclave PTB on the host
// without a Node-side Sui SDK.

const hex = (process.argv[2] || "").trim();
if (!/^[0-9a-fA-F]+$/.test(hex) || hex.length % 2 !== 0) {
    console.error("Usage: node hex-to-vector.mjs <even-length-hex-string>");
    process.exit(1);
}

const parts = new Array(hex.length / 2);
for (let i = 0; i < hex.length; i += 2) {
    parts[i / 2] = parseInt(hex.slice(i, i + 2), 16) + "u8";
}
process.stdout.write("[" + parts.join(",") + "]");
