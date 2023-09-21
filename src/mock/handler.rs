use crate::mock;
use crate::mock::session::Session;
use crate::mock::socket::Socket;
use crate::mock::{Action, Behaviour, CustomResponse, CustomResponseId, Expect, Request};
use discv5::enr::{CombinedKey, NodeId};
use discv5::handler::{NodeAddress, NodeContact};
use discv5::packet::{ChallengeData, IdNonce, MessageNonce, Packet, PacketKind};
use discv5::rpc::{Message, RequestBody};
use discv5::socket::{InboundPacket, OutboundPacket};
use discv5::{DefaultProtocolId, Enr, ListenConfig};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, UnboundedSender};
use tracing::{info, warn};

#[derive(Debug)]
/// A Challenge (WHOAREYOU) object used to handle and send WHOAREYOU requests.
pub struct Challenge {
    /// The challenge data received from the node.
    pub data: ChallengeData,
    /// The remote's ENR if we know it. We can receive a challenge from an unknown node.
    pub remote_enr: Option<Enr>,
}

pub(crate) enum HandlerIn {
    SendRandomPacket(NodeContact),
}

pub(crate) enum HandlerOut {}

pub(crate) struct Handler {
    enr: Enr,
    local_key: CombinedKey,
    node_id: NodeId,
    from_mock: mpsc::UnboundedReceiver<HandlerIn>,
    socket: Socket,
    behaviours: VecDeque<Behaviour>,
    active_challenges: HashMap<NodeAddress, Challenge>,
    sessions: HashMap<NodeAddress, Session>,
    captured_requests: Vec<discv5::rpc::Request>,
}

impl Handler {
    pub(crate) async fn spawn(
        enr: Enr,
        enr_key: CombinedKey,
        config: discv5::Config,
        behaviours: VecDeque<Behaviour>,
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
                    enr,
                    local_key: enr_key,
                    node_id,
                    from_mock,
                    socket,
                    behaviours,
                    active_challenges: HashMap::new(),
                    sessions: HashMap::new(),
                    captured_requests: vec![],
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
                    self.process_inbound_packet(inbound_packet).await;
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

    pub(crate) async fn process_inbound_packet(&mut self, inbound_packet: InboundPacket) {
        let inbound_packet_kind = inbound_packet.header.kind.clone();
        macro_rules! next_behaviour {
            () => {{
                self.behaviours.pop_front().expect(
                    format!("No behaviour. inbound_packet:{:?}", inbound_packet_kind).as_str(),
                )
            }};
        }

        match inbound_packet.header.kind.clone() {
            PacketKind::WhoAreYou { id_nonce, .. } => {
                let behaviour = next_behaviour!();
                match behaviour.expect {
                    Expect::WhoAreYou => {
                        info!("Received WHOAREYOU packet. id_nonce:{:?}", id_nonce)
                    }
                    _ => panic!(
                        "Unexpected inbound packet. expected:{:?}, actual:{:?}",
                        behaviour.expect, inbound_packet.header.kind
                    ),
                }

                match behaviour.action {
                    Action::Ignore(reason) => info!(
                        "Ignoring WHOAREYOU packet. id_nonce:{:?}, reason:{}",
                        id_nonce, reason
                    ),
                    Action::SendWhoAreYou => unreachable!(),
                    Action::EstablishSession(_) => unreachable!(),
                    Action::SendResponse(_) => unreachable!(),
                    Action::CaptureRequest => unreachable!(),
                }
            }
            PacketKind::Handshake {
                src_id,
                id_nonce_sig,
                ephem_pubkey,
                enr_record,
            } => {
                let behaviour = next_behaviour!();
                let expected_request_kind = match behaviour.expect {
                    Expect::Handshake(expected_request_kind) => {
                        info!("Received Handshake.");
                        expected_request_kind
                    }
                    _ => panic!(
                        "Unexpected inbound packet. expected:{:?}, actual:{:?}",
                        behaviour.expect, inbound_packet_kind
                    ),
                };

                match behaviour.action {
                    Action::Ignore(_) => todo!(),
                    Action::SendWhoAreYou => unreachable!(),
                    Action::EstablishSession(next_action) => {
                        let node_address = NodeAddress {
                            socket_addr: inbound_packet.src_address,
                            node_id: src_id,
                        };
                        if let Some(challenge) = self.active_challenges.remove(&node_address) {
                            self.establish_session(
                                node_address.clone(),
                                challenge,
                                &ephem_pubkey,
                                enr_record,
                            )
                            .await;

                            match next_action.as_ref() {
                                Action::Ignore(_) => {}
                                Action::SendWhoAreYou => {}
                                Action::EstablishSession(_) => {}
                                Action::SendResponse(_) => {}
                                Action::CaptureRequest => {
                                    self.capture_request(&node_address, &inbound_packet);
                                    // if let Some(session) = self.sessions.get(&node_address) {
                                    //     let message = decode_message(session, &inbound_packet);
                                    //     match message {
                                    //         Message::Request(request) => {
                                    //             self.captured_requests.push(request);
                                    //         }
                                    //         Message::Response(_) => unreachable!(),
                                    //     }
                                    // } else {
                                    //     panic!("Session does not exist.")
                                    // }
                                }
                            }
                        } else {
                            panic!("No active challenge");
                        }
                    }
                    Action::SendResponse(_) => unreachable!(),
                    Action::CaptureRequest => unreachable!(),
                }
            }
            PacketKind::Message { src_id } => {
                let node_address = NodeAddress {
                    socket_addr: inbound_packet.src_address,
                    node_id: src_id,
                };
                let behaviour = next_behaviour!();

                // Expect
                let mut received_request = None;
                match behaviour.expect {
                    Expect::MessageWithoutSession => {
                        // Check session existence
                        if self.sessions.contains_key(&node_address) {
                            panic!("Unexpected inbound packet. expected:MessageWithoutSession, actual:SessionExists");
                        }
                        info!("Received Message without session.");
                    }
                    Expect::Message(expected_request) => {
                        // Check if we have a session.
                        if let Some(session) = self.sessions.get(&node_address) {
                            match decode_message(session, &inbound_packet) {
                                Message::Request(request) => {
                                    if !check_request_kind(&request, &expected_request) {
                                        panic!("Unexpected request. {:?}", request);
                                    }

                                    received_request = Some(request);
                                }
                                Message::Response(_) => todo!(),
                            }
                        } else {
                            panic!("Session does not exist.")
                        }
                    }
                    _ => panic!(
                        "Unexpected inbound packet. expected:{:?}, actual:{:?}",
                        behaviour.expect, inbound_packet_kind
                    ),
                }

                // Action
                match behaviour.action {
                    Action::Ignore(reason) => info!("Ignoring Message packet. reason:{}", reason),
                    Action::SendWhoAreYou => {
                        let node_address = NodeAddress {
                            socket_addr: inbound_packet.src_address,
                            node_id: src_id,
                        };
                        self.send_challenge(node_address, inbound_packet.header.message_nonce)
                            .await;
                    }
                    Action::EstablishSession(_) => unreachable!(),
                    Action::SendResponse(response) => {
                        self.capture_request(&node_address, &inbound_packet);
                        match response {
                            mock::Response::Default => {
                                self.send_default_response(node_address, received_request.unwrap())
                                    .await
                            }
                            mock::Response::Custom(responses) => {
                                self.send_custom_responses(node_address, responses).await
                            }
                        }
                    },
                    Action::CaptureRequest => todo!(),
                }
            }
        }
    }

    fn capture_request(&mut self, node_address: &NodeAddress, inbound_packet: &InboundPacket) {
        if let Some(session) = self.sessions.get(node_address) {
            let message = decode_message(session, inbound_packet);
            match message {
                Message::Request(request) => {
                    self.captured_requests.push(request);
                }
                Message::Response(_) => unreachable!(),
            }
        } else {
            panic!("Session does not exist.")
        }
    }

    async fn send_challenge(&mut self, node_address: NodeAddress, message_nonce: MessageNonce) {
        let id_nonce: IdNonce = rand::random();
        let packet = Packet::new_whoareyou(message_nonce, id_nonce, 0);
        let challenge_data =
            ChallengeData::try_from(packet.authenticated_data::<DefaultProtocolId>().as_slice())
                .expect("Must be the correct challenge size");

        info!("Sending WHOAREYOU to {}", node_address);
        self.send(node_address.clone(), packet).await;
        if let Some(_) = self.active_challenges.insert(
            node_address,
            Challenge {
                data: challenge_data,
                remote_enr: None,
            },
        ) {
            panic!("Unexpected call for send_challenge()");
        }
    }

    async fn send_response(&mut self, node_address: NodeAddress, response: discv5::rpc::Response) {
        let packet = if let Some(session) = self.sessions.get_mut(&node_address) {
            session.encrypt_message(self.node_id, &response.encode())
        } else {
            return warn!(
                "Session is not established. Dropping response {} for node: {}",
                response, node_address.node_id
            );
        };

        match packet {
            Ok(packet) => self.send(node_address, packet).await,
            Err(e) => warn!("Could not encrypt response: {:?}", e),
        }
    }

    async fn send(&mut self, node_address: NodeAddress, packet: Packet) {
        let outbound_packet = OutboundPacket {
            node_address,
            packet,
        };
        self.socket.send.send(outbound_packet).await.unwrap();
    }

    async fn send_default_response(
        &mut self,
        node_address: NodeAddress,
        request: discv5::rpc::Request,
    ) {
        match request.body {
            RequestBody::Ping { .. } => {
                self.send_response(
                    node_address,
                    discv5::rpc::Response {
                        id: request.id,
                        body: discv5::rpc::ResponseBody::Pong {
                            enr_seq: self.enr.seq(),
                            ip: IpAddr::from(self.enr.ip4().unwrap()),
                            port: self.enr.udp4().unwrap(),
                        },
                    },
                )
                .await;
            }
            RequestBody::FindNode { .. } => todo!(),
            RequestBody::Talk { .. } => todo!(),
        }
    }

    async fn send_custom_responses(
        &mut self,
        node_address: NodeAddress,
        responses: Vec<CustomResponse>,
    ) {
        for res in responses {
            let id = match res.id {
                CustomResponseId::CapturedRequestId(index) => {
                    self.captured_requests.get(index).unwrap().id.clone()
                }
            };
            self.send_response(
                node_address.clone(),
                discv5::rpc::Response { id, body: res.body },
            )
            .await;
        }
    }

    async fn establish_session(
        &mut self,
        node_address: NodeAddress,
        challenge: Challenge,
        ephem_pubkey: &[u8],
        enr_record: Option<Enr>,
    ) {
        match Session::establish_from_challenge(
            &self.local_key,
            &self.node_id,
            &node_address.node_id,
            challenge,
            ephem_pubkey,
            enr_record,
        ) {
            Ok((session, _enr)) => {
                self.sessions.insert(node_address, session);
            }
            Err(error) => panic!("{}", error),
        }

        info!("Session established.");
    }
}

fn decode_message(session: &Session, inbound_packet: &InboundPacket) -> discv5::rpc::Message {
    // Decrypt the message
    let message = session
        .decrypt_message(
            inbound_packet.header.message_nonce.clone(),
            &inbound_packet.message,
            &inbound_packet.authenticated_data,
        )
        .expect("Decrypt message");

    discv5::rpc::Message::decode(&message).unwrap()
}

fn check_request_kind(request: &discv5::rpc::Request, expected: &Request) -> bool {
    match expected {
        Request::FINDNODE => match request.body {
            RequestBody::FindNode { .. } => true,
            _ => false,
        },
        Request::Ping => match request.body {
            RequestBody::Ping { .. } => true,
            _ => false,
        },
    }
}
