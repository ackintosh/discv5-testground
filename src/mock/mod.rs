mod crypto;
mod ecdh;
mod handler;
mod session;
mod socket;

use crate::mock::handler::{Handler, HandlerIn};
use crate::mock::session::Session;
use discv5::enr::CombinedKey;
use discv5::handler::{NodeAddress, NodeContact};
use discv5::{Enr, IpMode};
use std::collections::{HashMap, VecDeque};
use tokio::sync::mpsc;
use tracing::info;

pub struct Behaviour {
    pub expect: Expect,
    pub action: Action,
}

#[derive(Debug)]
pub enum Expect {
    WhoAreYou,
    MessageWithoutSession,
    Handshake(Request),
    Message(Request),
}

#[derive(Debug)]
pub enum Request {
    FINDNODE,
    Ping,
}

pub enum Action {
    Ignore(String),
    SendWhoAreYou,
    EstablishSession(Box<Action>),
}

pub(crate) struct Mock {
    /// The channel to send messages to the handler.
    to_handler: mpsc::UnboundedSender<HandlerIn>,
}

impl Mock {
    pub(crate) async fn start(
        enr: Enr,
        enr_key: CombinedKey,
        config: discv5::Config,
        behaviours: VecDeque<Behaviour>,
    ) -> Self {
        let (to_handler, _from_handler) = Handler::spawn(enr, enr_key, config, behaviours).await;

        Mock { to_handler }
    }

    pub(crate) fn send_random_packet(&mut self, enr: Enr) -> Result<(), String> {
        let node_contact = NodeContact::try_from_enr(enr, IpMode::Ip4).unwrap();
        info!(
            "Sending random packet to {} {}",
            node_contact.node_id(),
            node_contact.socket_addr()
        );
        self.to_handler
            .send(HandlerIn::SendRandomPacket(node_contact))
            .map_err(|e| format!("Failed to send message to the handler: {e}"))?;

        Ok(())
    }
}
