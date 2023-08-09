use crate::mock::socket::Socket;
use discv5::enr::NodeId;
use discv5::handler::NodeContact;
use discv5::packet::{Packet, PacketKind};
use discv5::socket::{InboundPacket, OutboundPacket};
use discv5::{Discv5Config, Enr};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, UnboundedSender};
use tracing::{info, warn};

pub(crate) enum HandlerIn {
    SendRandomPacket(NodeContact),
}

pub(crate) enum HandlerOut {}

pub(crate) struct Handler {
    node_id: NodeId,
    from_mock: mpsc::UnboundedReceiver<HandlerIn>,
    socket: Socket,
}

impl Handler {
    pub(crate) async fn spawn(
        enr: Enr,
        config: Discv5Config,
    ) -> (UnboundedSender<HandlerIn>, Receiver<HandlerOut>) {
        let (handler_send, from_mock) = mpsc::unbounded_channel();
        let (_to_mock, handler_recv) = mpsc::channel(50);

        let node_id = enr.node_id();

        let socket = Socket::new(
            config.executor.clone().expect("Executor must exist"),
            node_id,
            config.listen_config.clone(),
        )
        .await;

        config
            .executor
            .clone()
            .expect("Executor must be present")
            .spawn(Box::pin(async move {
                let mut handler = Handler {
                    node_id,
                    from_mock,
                    socket,
                };

                handler.start().await;
            }));

        (handler_send, handler_recv)
    }

    pub(crate) async fn start(&mut self) {
        loop {
            tokio::select! {
                Some(handler_request) = self.from_mock.recv() => {
                    self.process_handler_request(handler_request).await;
                }
                Some(inbound_packet) = self.socket.recv.recv() => {
                    self.process_inbound_packet(inbound_packet);
                }
            }
        }
    }

    pub(crate) async fn process_handler_request(&self, handler_request: HandlerIn) {
        match handler_request {
            HandlerIn::SendRandomPacket(node_contact) => {
                let packet = Packet::new_random(&self.node_id).unwrap();
                let outbound_packet = OutboundPacket {
                    node_address: node_contact.node_address(),
                    packet,
                };

                if let Err(e) = self.socket.send.send(outbound_packet).await {
                    warn!("Failed to send OutboundPacket to SendHandler: {e}");
                }
            }
        }
    }

    pub(crate) fn process_inbound_packet(&self, inbound_packet: InboundPacket) {
        match inbound_packet.header.kind {
            PacketKind::Message { .. } => todo!(),
            PacketKind::WhoAreYou { .. } => {
                info!("Received WHOAREYOU packet but dropped it without replying.")
            }
            PacketKind::Handshake { .. } => todo!(),
        }
    }
}
