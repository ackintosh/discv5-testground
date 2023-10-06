mod crypto;
mod ecdh;
mod handler;
mod session;
mod socket;

use crate::mock::handler::{Handler, HandlerIn};
use discv5::enr::CombinedKey;
use discv5::handler::NodeContact;
use discv5::{Enr, IpMode};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tracing::info;

pub enum Behaviours {
    Declarative(DeclarativeBehaviour),
    Sequential(VecDeque<Behaviour>),
}

pub struct DeclarativeBehaviour {
    pub whoareyou: Vec<Action>,
    pub handshake: Vec<Action>,
    pub message: Vec<Action>,
    pub message_without_session: Vec<Action>,
}

pub struct Behaviour {
    pub expect: Expect,
    pub actions: Vec<Action>,
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

#[derive(Clone)]
pub enum Action {
    Ignore(String),
    SendWhoAreYou,
    EstablishSession,
    SendResponse(Response),
    CaptureRequest,
}

#[derive(Clone)]
pub enum Response {
    Default,
    Custom(Vec<CustomResponse>),
}

#[derive(Clone)]
pub enum CustomResponseId {
    CapturedRequestId(usize),
}

#[derive(Clone)]
pub struct CustomResponse {
    pub id: CustomResponseId,
    pub body: discv5::rpc::ResponseBody,
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
        behaviours: Behaviours,
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
