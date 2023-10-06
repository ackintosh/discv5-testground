use crate::mock::{
    Action, Behaviour, Behaviours, CustomResponse, CustomResponseId, DeclarativeBehaviour, Expect,
    Mock, Request, Response,
};
use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::rpc::ResponseBody;
use discv5::{Discv5, Enr, ListenConfig};
use enr::{k256, NodeId};
use rand::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use std::vec;
use testground::client::Client;
use tracing::{error, info};

const STATE_DISCV5_STARTED: &str = "state_discv5_started";
const STATE_FINISHED: &str = "state_finished";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr,
}

pub(crate) async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let run_parameters = client.run_parameters();
    let ip = run_parameters
        .data_network_ip()?
        .expect("IP address for the data network");

    // Seed is chosen such that all nodes are in the 256th bucket of bootstrap
    let seed = 1652;
    let mut keypairs = generate_deterministic_keypair(5, seed);

    let target_enr = {
        let target_key = keypairs.pop().unwrap();
        EnrBuilder::new("v4").build(&target_key).expect("enr")
    };

    // ////////////////////////
    // Construct local Enr
    // ////////////////////////
    let enr_key = keypairs.remove(client.global_seq() as usize - 1);
    let enr = EnrBuilder::new("v4")
        .ip(ip)
        .udp4(9000)
        .build(&enr_key)
        .expect("enr");

    // //////////////////////////////////////////////////////////////
    // Collect information of all participants in the test case
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo {
        seq: client.global_seq(),
        enr: enr.clone(),
    };
    client.record_message(format!(
        "seq: {}, node_id: {}, ip: {}",
        instance_info.seq,
        instance_info.enr.node_id(),
        ip
    ));

    let participants = publish_and_collect(&client, instance_info).await?;

    // let another_instance_info = {
    //     let participants = publish_and_collect(&client, instance_info).await?;
    //
    //     if client.global_seq() == 1 {
    //         participants
    //             .into_iter()
    //             .find(|p| p.seq == 2).unwrap()
    //     } else {
    //         participants
    //             .into_iter()
    //             .find(|p| p.seq == 1)
    //             .unwrap()
    //     }
    // };

    // ////////////////////////
    // Discv5 config
    // ////////////////////////
    let listen_config = ListenConfig::Ipv4 {
        ip: Ipv4Addr::UNSPECIFIED,
        port: 9000,
    };
    let config = discv5::ConfigBuilder::new(listen_config)
        .vote_duration(Duration::from_secs(3))
        .enr_peer_update_min(2)
        .request_timeout(Duration::from_secs(3))
        .build();

    match client.global_seq() {
        1 => {
            run_discv5(
                client,
                enr,
                enr_key,
                config,
                participants,
                target_enr.node_id(),
            )
            .await?
        }
        2 | 3 => run_mock(client, enr, enr_key, config, participants, target_enr).await?,
        _ => unreachable!(),
    }

    Ok(())
}

async fn run_discv5(
    client: Client,
    enr: Enr,
    enr_key: CombinedKey,
    config: discv5::Config,
    participants: Vec<InstanceInfo>,
    _target_node_id: NodeId,
) -> Result<(), Box<dyn std::error::Error>> {
    // ////////////////////////
    // Start discv5
    // ////////////////////////
    let mut discv5: Discv5 = Discv5::new(enr.clone(), enr_key, config)?;
    for peer in participants.iter().filter(|p| p.seq != client.global_seq()) {
        discv5.add_enr(peer.enr.clone()).unwrap();
    }
    discv5.start().await.expect("Start Discovery v5 server");

    client
        .signal_and_wait(
            STATE_DISCV5_STARTED,
            client.run_parameters().test_instance_count,
        )
        .await?;

    let mut handles = vec![];
    for peer in participants.iter().filter(|p| p.seq != client.global_seq()) {
        for _ in 0..2 {
            let fut = discv5.send_ping(peer.enr.clone());
            // let fut = discv5.find_node(target_node_id);
            handles.push(tokio::spawn(fut));
        }
    }

    let mut succeeded = true;
    for h in handles {
        match h.await.unwrap() {
            Ok(res) => info!("Response: {:?}", res),
            Err(e) => {
                succeeded = false;
                error!("Request failed: {e}");
            }
        }
    }

    tokio::time::sleep(Duration::from_secs(10)).await;

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    if succeeded {
        client.record_success().await?;
    } else {
        client.record_failure("").await?;
    }
    Ok(())
}

async fn run_mock(
    client: Client,
    enr: Enr,
    enr_key: CombinedKey,
    config: discv5::Config,
    participants: Vec<InstanceInfo>,
    _target_enr: Enr,
) -> Result<(), Box<dyn std::error::Error>> {
    let discv5_node = participants.into_iter().find(|p| p.seq == 1).unwrap();
    // ////////////////////////
    // Start mock
    // ////////////////////////
    // Sequential: FINDNODE
    // let mut behaviours = VecDeque::new();
    // behaviours.push_back(Behaviour {
    //     expect: Expect::Handshake(Request::FINDNODE),
    //     action: Action::EstablishSession(Box::new(Action::CaptureRequest)),
    // });
    // behaviours.push_back(Behaviour {
    //     expect: Expect::Message(Request::FINDNODE),
    //     action: Action::SendResponse(Response::Custom(vec![
    //         CustomResponse {
    //             id: CustomResponseId::CapturedRequestId(0),
    //             body: discv5::rpc::ResponseBody::Nodes {
    //                 total: 2,
    //                 nodes: vec![target_enr.clone()],
    //             },
    //         },
    //         CustomResponse {
    //             id: CustomResponseId::CapturedRequestId(1),
    //             body: discv5::rpc::ResponseBody::Nodes {
    //                 total: 1,
    //                 nodes: vec![target_enr.clone()],
    //             },
    //         },
    //         CustomResponse {
    //             id: CustomResponseId::CapturedRequestId(0),
    //             body: discv5::rpc::ResponseBody::Nodes {
    //                 total: 2,
    //                 nodes: vec![target_enr],
    //             },
    //         },
    //     ])),
    // });

    // Sequential: PING
    // let mut behaviours = VecDeque::new();
    // behaviours.push_back(Behaviour {
    //     expect: Expect::MessageWithoutSession,
    //     actions: vec![Action::SendWhoAreYou],
    // });
    // behaviours.push_back(Behaviour {
    //     expect: Expect::Handshake(Request::Ping),
    //     actions: vec![
    //         Action::EstablishSession,
    //         Action::CaptureRequest,
    //         Action::SendResponse(Response::Custom(vec![CustomResponse {
    //             id: CustomResponseId::CapturedRequestId(0),
    //             body: ResponseBody::Pong {
    //                 enr_seq: discv5_node.enr.seq(),
    //                 ip: IpAddr::V4(discv5_node.enr.ip4().unwrap()),
    //                 port: 0, // test
    //             },
    //         }])),
    //     ],
    // });
    // behaviours.push_back(Behaviour {
    //     expect: Expect::Message(Request::Ping),
    //     actions: vec![
    //         Action::CaptureRequest,
    //         Action::SendResponse(Response::Custom(vec![CustomResponse {
    //             id: CustomResponseId::CapturedRequestId(1),
    //             body: ResponseBody::Pong {
    //                 enr_seq: discv5_node.enr.seq(),
    //                 ip: IpAddr::V4(discv5_node.enr.ip4().unwrap()),
    //                 port: 0, // test
    //             },
    //         }])),
    //     ],
    // });

    // Declarative
    let behaviours = Behaviours::Declarative(DeclarativeBehaviour {
        whoareyou: vec![],
        message_without_session: vec![Action::SendWhoAreYou],
        handshake: vec![Action::Ignore("Ignoring handshake messages".to_string())],
        message: vec![Action::SendWhoAreYou],
    });
    let mut _mock = Mock::start(enr, enr_key, config, behaviours).await;
    // let mut _mock = Mock::start(enr, enr_key, config, Behaviours::Sequential(behaviours)).await;

    client
        .signal_and_wait(
            STATE_DISCV5_STARTED,
            client.run_parameters().test_instance_count,
        )
        .await?;

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    client.record_success().await?;
    Ok(())
}

/// Generate `n` deterministic keypairs from a given seed.
fn generate_deterministic_keypair(n: usize, seed: u64) -> Vec<CombinedKey> {
    let mut keypairs = Vec::new();
    for i in 0..n {
        let sk = {
            let rng = &mut rand_xorshift::XorShiftRng::seed_from_u64(seed + i as u64);
            let mut b = [0; 32];
            loop {
                // until a value is given within the curve order
                rng.fill_bytes(&mut b);
                if let Ok(k) = k256::ecdsa::SigningKey::from_slice(&b) {
                    break k;
                }
            }
        };
        let kp = CombinedKey::from(sk);
        keypairs.push(kp);
    }
    keypairs
}
