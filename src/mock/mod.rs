mod handler;
mod socket;

use crate::mock::handler::{Handler, HandlerIn, HandlerOut};
use discv5::handler::NodeContact;
use discv5::{Discv5Config, Enr, IpMode};
use tokio::sync::mpsc;
use tracing::info;

pub(crate) struct Mock {
    config: Discv5Config,
    enr: Enr,
    /// The channel to send messages to the handler.
    handler_send: mpsc::UnboundedSender<HandlerIn>,
    /// The channel to receive messages from the handler.
    handler_recv: mpsc::Receiver<HandlerOut>,
}

impl Mock {
    pub(crate) async fn start(enr: Enr, config: Discv5Config) -> Self {
        let (handler_send, handler_recv) = Handler::spawn(enr.clone(), config.clone()).await;

        Mock {
            config,
            enr,
            handler_send,
            handler_recv,
        }
    }

    pub(crate) fn send_random_packet(&mut self, enr: Enr) -> Result<(), String> {
        let node_contact = NodeContact::try_from_enr(enr, IpMode::Ip4).unwrap();
        info!(
            "Sending random packet to {} {}",
            node_contact.node_id(),
            node_contact.socket_addr()
        );
        self.handler_send
            .send(HandlerIn::SendRandomPacket(node_contact))
            .map_err(|e| format!("Failed to send message to the handler: {e}"))?;

        Ok(())
    }
}
