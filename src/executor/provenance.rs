use sha3::{Digest, Sha3_256, Sha3_512};

/// SHA3-512 over a string — used for full evidence-record provenance.
pub fn sha3_hex(data: &str) -> String {
    let mut hasher = Sha3_512::new();
    hasher.update(data.as_bytes());
    format!("sha3-512:{}", hex::encode(hasher.finalize()))
}

/// SHA3-512 over raw bytes.
pub fn sha3_bytes(data: &[u8]) -> String {
    let mut hasher = Sha3_512::new();
    hasher.update(data);
    format!("sha3-512:{}", hex::encode(hasher.finalize()))
}

/// SHA3-256 over raw bytes — used to pin test attachments (images).
/// Matches the `sha3-256:<hex>` format stored in tests.attachment_sha3.
pub fn sha3_256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    format!("sha3-256:{}", hex::encode(hasher.finalize()))
}
