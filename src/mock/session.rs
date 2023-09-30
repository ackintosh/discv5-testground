use crate::mock::crypto::derive_keys_from_pubkey;
use crate::mock::handler::Challenge;
use discv5::enr::{CombinedKey, NodeId};
use discv5::packet::{MessageNonce, Packet, PacketHeader, PacketKind};
use discv5::{DefaultProtocolId, Enr};
use zeroize::Zeroize;

/// The message nonce length (in bytes).
pub const MESSAGE_NONCE_LENGTH: usize = 12;

#[derive(Zeroize, PartialEq)]
pub(crate) struct Keys {
    /// The encryption key.
    encryption_key: [u8; 16],
    /// The decryption key.
    decryption_key: [u8; 16],
}

pub(crate) struct Session {
    keys: Keys,
    counter: u32,
}

impl Session {
    pub(crate) fn new(keys: Keys) -> Session {
        Session { keys, counter: 0 }
    }

    pub(crate) fn establish_from_challenge(
        local_key: &CombinedKey,
        local_id: &NodeId,
        remote_id: &NodeId,
        challenge: &Challenge,
        // id_nonce_sig: &[u8],
        ephem_pubkey: &[u8],
        enr_record: Option<Enr>,
    ) -> Result<(Session, Enr), String> {
        // generate session keys
        let (decryption_key, encryption_key) = derive_keys_from_pubkey(
            local_key,
            local_id,
            remote_id,
            &challenge.data,
            ephem_pubkey,
        )?;

        let keys = Keys {
            encryption_key,
            decryption_key,
        };

        Ok((Session::new(keys), enr_record.unwrap()))
    }

    pub(crate) fn encrypt_message(
        &mut self,
        src_id: NodeId,
        message: &[u8],
    ) -> Result<Packet, String> {
        self.counter += 1;

        // If the message nonce length is ever set below 4 bytes this will explode. The packet
        // size constants shouldn't be modified.
        let random_nonce: [u8; MESSAGE_NONCE_LENGTH - 4] = rand::random();
        let mut message_nonce: MessageNonce = [0u8; MESSAGE_NONCE_LENGTH];
        message_nonce[..4].copy_from_slice(&self.counter.to_be_bytes());
        message_nonce[4..].copy_from_slice(&random_nonce);

        // the authenticated data is the IV concatenated with the packet header
        let iv: u128 = rand::random();
        let header = PacketHeader {
            message_nonce,
            kind: PacketKind::Message { src_id },
        };

        let mut authenticated_data = iv.to_be_bytes().to_vec();
        authenticated_data.extend_from_slice(&header.encode::<DefaultProtocolId>());

        let cipher = crate::mock::crypto::encrypt_message(
            &self.keys.encryption_key,
            message_nonce,
            message,
            &authenticated_data,
        )?;

        // construct a packet from the header and the cipher text
        Ok(Packet {
            iv,
            header,
            message: cipher,
        })
    }

    pub(crate) fn decrypt_message(
        &self,
        message_nonce: MessageNonce,
        message: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, String> {
        crate::mock::crypto::decrypt_message(&self.keys.decryption_key, message_nonce, message, aad)
    }
}
