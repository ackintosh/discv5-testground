use crate::mock::ecdh::ecdh;
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{Aead, NewAead, Payload};
use aes_gcm::Aes128Gcm;
use discv5::enr::k256::sha2::Sha256;
use discv5::enr::{k256, CombinedKey, NodeId};
use discv5::packet::{ChallengeData, MessageNonce};
use hkdf::Hkdf;

const NODE_ID_LENGTH: usize = 32;
const INFO_LENGTH: usize = 26 + 2 * NODE_ID_LENGTH;
const KEY_LENGTH: usize = 16;
const KEY_AGREEMENT_STRING: &str = "discovery v5 key agreement";

type Key = [u8; KEY_LENGTH];

/// Derives the session keys for a public key type that matches the local keypair.
pub(crate) fn derive_keys_from_pubkey(
    local_key: &CombinedKey,
    local_id: &NodeId,
    remote_id: &NodeId,
    challenge_data: &ChallengeData,
    ephem_pubkey: &[u8],
) -> Result<(Key, Key), String> {
    let secret = {
        match local_key {
            CombinedKey::Secp256k1(key) => {
                // convert remote pubkey into secp256k1 public key
                // the key type should match our own node record
                let remote_pubkey = k256::ecdsa::VerifyingKey::from_sec1_bytes(ephem_pubkey)
                    .map_err(|_| "Error::InvalidRemotePublicKey".to_string())?;
                ecdh(&remote_pubkey, key)
            }
            CombinedKey::Ed25519(_) => {
                return Err("Error::KeyTypeNotSupported(Ed25519)".to_string())
            }
        }
    };

    derive_key(&secret, remote_id, local_id, challenge_data)
}

fn derive_key(
    secret: &[u8],
    first_id: &NodeId,
    second_id: &NodeId,
    challenge_data: &ChallengeData,
) -> Result<(Key, Key), String> {
    let mut info = [0u8; INFO_LENGTH];
    info[0..26].copy_from_slice(KEY_AGREEMENT_STRING.as_bytes());
    info[26..26 + NODE_ID_LENGTH].copy_from_slice(&first_id.raw());
    info[26 + NODE_ID_LENGTH..].copy_from_slice(&second_id.raw());

    let hk = Hkdf::<Sha256>::new(Some(challenge_data.as_ref()), secret);

    let mut okm = [0u8; 2 * KEY_LENGTH];
    hk.expand(&info, &mut okm)
        .map_err(|_| "Error::KeyDerivationFailed".to_string())?;

    let mut initiator_key: Key = Default::default();
    let mut recipient_key: Key = Default::default();
    initiator_key.copy_from_slice(&okm[0..KEY_LENGTH]);
    recipient_key.copy_from_slice(&okm[KEY_LENGTH..2 * KEY_LENGTH]);

    Ok((initiator_key, recipient_key))
}

pub(crate) fn encrypt_message(
    key: &Key,
    message_nonce: MessageNonce,
    msg: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    let aead = Aes128Gcm::new(GenericArray::from_slice(key));
    let payload = Payload { msg, aad };
    aead.encrypt(GenericArray::from_slice(&message_nonce), payload)
        .map_err(|e| e.to_string())
}

pub(crate) fn decrypt_message(
    key: &Key,
    message_nonce: MessageNonce,
    msg: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    if msg.len() < 16 {
        return Err("Message not long enough to contain a MAC".to_string());
    }

    let aead = Aes128Gcm::new(GenericArray::from_slice(key));
    let payload = Payload { msg, aad };
    aead.decrypt(GenericArray::from_slice(&message_nonce), payload)
        .map_err(|e| e.to_string())
}
