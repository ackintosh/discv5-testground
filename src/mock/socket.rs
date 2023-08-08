use discv5::enr::NodeId;
use discv5::packet::Packet;
use discv5::socket::{InboundPacket, OutboundPacket};
use discv5::{DefaultProtocolId, Executor, ListenConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::warn;

pub(crate) const MAX_PACKET_SIZE: usize = 1280;

pub(crate) struct Socket {
    pub recv: Receiver<InboundPacket>,
    pub send: Sender<OutboundPacket>,
}

impl Socket {
    pub(crate) async fn new(
        executor: Box<dyn Executor + Send + Sync>,
        node_id: NodeId,
        listen_config: ListenConfig,
    ) -> Self {
        let socket = match listen_config {
            ListenConfig::Ipv4 { ip, port } => {
                let socket_addr: SocketAddr = (ip, port).into();
                Arc::new(UdpSocket::bind(socket_addr).await.unwrap())
            }
            ListenConfig::Ipv6 { .. } => unreachable!(),
            ListenConfig::DualStack { .. } => unreachable!(),
        };

        let from_recv_handler = RecvHandler::spawn(executor.clone(), node_id, socket.clone());
        let to_send_handler = SendHandler::spawn(executor, socket);

        Socket {
            recv: from_recv_handler,
            send: to_send_handler,
        }
    }
}

struct RecvHandler {
    node_id: NodeId,
    socket: Arc<UdpSocket>,
    handler_send: mpsc::Sender<InboundPacket>,
}

impl RecvHandler {
    pub(crate) fn spawn(
        executor: Box<dyn Executor>,
        node_id: NodeId,
        socket: Arc<UdpSocket>,
    ) -> Receiver<InboundPacket> {
        // create the channel to send decoded packets to the handler
        let (handler_send, handler_recv) = mpsc::channel(30);

        let receive_handler = RecvHandler {
            node_id,
            socket,
            handler_send,
        };

        executor.spawn(Box::pin(async move {
            receive_handler.start().await;
        }));

        handler_recv
    }

    async fn start(&self) {
        loop {
            let mut first_buffer = [0; MAX_PACKET_SIZE];

            if let Ok((length, src_address)) = self.socket.recv_from(&mut first_buffer).await {
                // self.handle_inbound(src, length, &first_buffer).await;
                let (packet, authenticated_data) = match Packet::decode::<DefaultProtocolId>(
                    &self.node_id,
                    &first_buffer[..length],
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("Failed to decode packet: {e:?}");
                        continue;
                    }
                };

                let inbound = InboundPacket {
                    src_address,
                    header: packet.header,
                    message: packet.message,
                    authenticated_data,
                };

                self.handler_send.send(inbound).await.unwrap_or_else(|e| {
                    warn!("Could not send packet to handler: {e:?}");
                })
            }
        }
    }
}

struct SendHandler {
    from_handler: Receiver<OutboundPacket>,
    socket: Arc<UdpSocket>,
}

impl SendHandler {
    pub(crate) fn spawn(
        executor: Box<dyn Executor>,
        socket: Arc<UdpSocket>,
    ) -> Sender<OutboundPacket> {
        let (to_send_handler, from_handler) = mpsc::channel(30);

        let mut send_handler = SendHandler {
            from_handler,
            socket,
        };

        executor.spawn(Box::pin(async move {
            send_handler.start().await;
        }));

        to_send_handler
    }

    async fn start(&mut self) {
        loop {
            if let Some(outbound_packet) = self.from_handler.recv().await {
                let encoded_packet = outbound_packet
                    .packet
                    .encode::<DefaultProtocolId>(&outbound_packet.node_address.node_id);
                let dest = &outbound_packet.node_address.socket_addr;
                let _ = self.socket.send_to(&encoded_packet, dest).await.unwrap();
            }
        }
    }
}
