mod handler;
mod socket;

use crate::mock::handler::{Handler, HandlerIn};
use discv5::handler::NodeContact;
use discv5::{Discv5Config, Enr, IpMode};
use tokio::sync::mpsc;
use tracing::info;

pub(crate) struct Mock {
    /// The channel to send messages to the handler.
    to_handler: mpsc::UnboundedSender<HandlerIn>,
}

impl Mock {
    pub(crate) async fn start(enr: Enr, config: Discv5Config) -> Self {
        let (to_handler, _from_handler) = Handler::spawn(enr, config).await;

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
