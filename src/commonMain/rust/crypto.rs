use iroh::{PublicKey, SecretKey, Signature};

use crate::error::IrohError;

/// A freshly generated Ed25519 keypair as raw bytes (32 bytes each), for callers
/// that need to mint a device/root key without going through [`crate::IrohEndpoint::bind`].
/// UniFFI free functions can't return a bare tuple, hence the record.
#[derive(uniffi::Record)]
pub struct Ed25519Keypair {
    pub secret: Vec<u8>,
    pub public: Vec<u8>,
}

/// Generate a fresh random Ed25519 keypair. Uses the same RNG path as
/// [`crate::IrohEndpoint::bind`]'s implicit key generation (`iroh::SecretKey::generate`,
/// i.e. `rand::random()`), so no extra crypto/RNG dependency is added for this crate.
#[uniffi::export]
pub fn generate_ed25519_keypair() -> Ed25519Keypair {
    let secret = SecretKey::generate();
    let public = secret.public();
    Ed25519Keypair {
        secret: secret.to_bytes().to_vec(),
        public: public.as_bytes().to_vec(),
    }
}

/// Ed25519-sign `data` with a raw 32-byte `secret`, returning the raw 64-byte
/// signature. Errors if `secret` isn't exactly 32 bytes. Unlike
/// [`crate::IrohEndpoint::sign`], this doesn't require a bound endpoint — for signing
/// with a root/device key that never itself binds an endpoint (see azula-docs
/// `multi-device-identity`).
#[uniffi::export]
pub fn ed25519_sign(secret: Vec<u8>, data: Vec<u8>) -> Result<Vec<u8>, IrohError> {
    let key = SecretKey::try_from(secret.as_slice()).map_err(IrohError::msg)?;
    Ok(key.sign(&data).to_bytes().to_vec())
}

/// Verify an Ed25519 `signature` over `data` against a raw 32-byte `public` key.
/// Never throws: a malformed public key or signature (wrong length or otherwise
/// invalid) is just an invalid signature (`false`), mirroring [`crate::verify_signature`].
#[uniffi::export]
pub fn ed25519_verify(public: Vec<u8>, data: Vec<u8>, signature: Vec<u8>) -> bool {
    let Ok(key) = PublicKey::try_from(public.as_slice()) else {
        return false;
    };
    let Ok(sig) = Signature::try_from(signature.as_slice()) else {
        return false;
    };
    key.verify(&data, &sig).is_ok()
}

/// Derive the raw 32-byte public key for a raw 32-byte `secret`. Errors if
/// `secret` isn't exactly 32 bytes.
#[uniffi::export]
pub fn ed25519_public_from_secret(secret: Vec<u8>) -> Result<Vec<u8>, IrohError> {
    let key = SecretKey::try_from(secret.as_slice()).map_err(IrohError::msg)?;
    Ok(key.public().as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// RFC 8032 §7.1 TEST 1: 0-byte message (test-only key, never used for real
    /// identity — see azula-docs `invitations.md` / `multi-device-identity`).
    #[test]
    fn rfc8032_test1() {
        let secret = hex_decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let public = hex_decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let message = Vec::new();
        let signature = hex_decode(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );

        assert_eq!(ed25519_public_from_secret(secret.clone()).expect("public_from_secret"), public);
        let sig = ed25519_sign(secret, message.clone()).expect("sign");
        assert_eq!(sig, signature);
        assert!(ed25519_verify(public, message, sig));
    }

    /// RFC 8032 §7.1 TEST 2: 1-byte message.
    #[test]
    fn rfc8032_test2() {
        let secret = hex_decode("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb");
        let public = hex_decode("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
        let message = hex_decode("72");
        let signature = hex_decode(
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        );

        assert_eq!(ed25519_public_from_secret(secret.clone()).expect("public_from_secret"), public);
        let sig = ed25519_sign(secret, message.clone()).expect("sign");
        assert_eq!(sig, signature);
        assert!(ed25519_verify(public, message, sig));
    }

    /// RFC 8032 §7.1 TEST 3: 2-byte message.
    #[test]
    fn rfc8032_test3() {
        let secret = hex_decode("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7");
        let public = hex_decode("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025");
        let message = hex_decode("af82");
        let signature = hex_decode(
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        );

        assert_eq!(ed25519_public_from_secret(secret.clone()).expect("public_from_secret"), public);
        let sig = ed25519_sign(secret, message.clone()).expect("sign");
        assert_eq!(sig, signature);
        assert!(ed25519_verify(public, message, sig));
    }

    #[test]
    fn generate_sign_verify_roundtrip() {
        let keypair = generate_ed25519_keypair();
        assert_eq!(keypair.secret.len(), 32);
        assert_eq!(keypair.public.len(), 32);
        assert_eq!(ed25519_public_from_secret(keypair.secret.clone()).expect("public_from_secret"), keypair.public);

        let data = b"hello multi-device-identity".to_vec();
        let sig = ed25519_sign(keypair.secret.clone(), data.clone()).expect("sign");
        assert_eq!(sig.len(), 64);
        assert!(ed25519_verify(keypair.public.clone(), data.clone(), sig.clone()));

        // Two keypairs shouldn't collide.
        let other = generate_ed25519_keypair();
        assert_ne!(keypair.secret, other.secret);
        assert_ne!(keypair.public, other.public);

        // Tampered signature, tampered data, and wrong public key all fail.
        let mut bad_sig = sig.clone();
        bad_sig[0] ^= 0x01;
        assert!(!ed25519_verify(keypair.public.clone(), data.clone(), bad_sig));

        let mut bad_data = data.clone();
        bad_data[0] ^= 0x01;
        assert!(!ed25519_verify(keypair.public.clone(), bad_data, sig.clone()));

        assert!(!ed25519_verify(other.public, data, sig));
    }

    #[test]
    fn wrong_length_secret_errors() {
        assert!(ed25519_sign(vec![0u8; 31], b"x".to_vec()).is_err());
        assert!(ed25519_sign(vec![0u8; 33], b"x".to_vec()).is_err());
        assert!(ed25519_public_from_secret(Vec::new()).is_err());
    }

    #[test]
    fn malformed_verify_inputs_return_false_not_panic() {
        let keypair = generate_ed25519_keypair();
        let data = b"data".to_vec();
        let sig = ed25519_sign(keypair.secret.clone(), data.clone()).expect("sign");

        // Wrong-length public key.
        assert!(!ed25519_verify(vec![0u8; 31], data.clone(), sig.clone()));
        assert!(!ed25519_verify(Vec::new(), data.clone(), sig.clone()));
        // Wrong-length signature.
        assert!(!ed25519_verify(keypair.public.clone(), data.clone(), vec![0u8; 63]));
        assert!(!ed25519_verify(keypair.public, data, Vec::new()));
    }
}
