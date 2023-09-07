use discv5::Enr;
use discv5::enr::{CombinedKey, NodeId};
use zeroize::Zeroize;
use crate::mock::crypto::derive_keys_from_pubkey;
use crate::mock::handler::Challenge;

#[derive(Zeroize, PartialEq)]
pub(crate) struct Keys {
    /// The encryption key.
    encryption_key: [u8; 16],
    /// The decryption key.
    decryption_key: [u8; 16],
}

pub(crate) struct Session {
    keys: Keys,
}

impl Session {
    pub(crate) fn new(keys: Keys) -> Session {
        Session { keys }
    }

    pub(crate) fn establish_from_challenge(
        local_key: &CombinedKey,
        local_id: &NodeId,
        remote_id: &NodeId,
        challenge: Challenge,
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
}
