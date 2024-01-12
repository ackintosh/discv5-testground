use crate::concurrent_requests::InstanceInfo;
use crate::mock::{Action, Behaviour, Behaviours, Expect, Mock, Request};
use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::{Discv5, Enr, ListenConfig};
use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::Duration;
use testground::client::Client;
use tracing::error;

const STATE_DISCV5_STARTED: &str = "state_discv5_started";
const STATE_SENT_RANDOM_PACKET: &str = "state_sent_random_packet";
const STATE_FINISHED: &str = "state_finished";

pub(crate) async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let run_parameters = client.run_parameters();
    let ip = run_parameters
        .data_network_ip()?
        .expect("IP address for the data network");

    // ////////////////////////
    // Construct local Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();
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

    let another_instance_info = {
        let participants = publish_and_collect(&client, instance_info).await?;
        assert_eq!(2, participants.len());

        let info = participants
            .into_iter()
            .filter(|p| p.seq != client.global_seq())
            .collect::<Vec<_>>();
        assert!(!info.is_empty());

        info.first().unwrap().clone()
    };

    // ////////////////////////
    // Discv5 config
    // ////////////////////////
    let listen_config = ListenConfig::Ipv4 {
        ip: Ipv4Addr::UNSPECIFIED,
        port: 9000,
    };
    let config = discv5::ConfigBuilder::new(listen_config)
        .request_timeout(Duration::from_secs(5))
        .build();

    match client.global_seq() {
        1 => run_discv5(client, enr, enr_key, config, another_instance_info).await?,
        2 => run_mock(client, enr, enr_key, config, another_instance_info).await?,
        _ => unreachable!(),
    }

    Ok(())
}

async fn run_discv5(
    client: Client,
    enr: Enr,
    enr_key: CombinedKey,
    config: discv5::Config,
    another_instance_info: InstanceInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    // ////////////////////////
    // Start discv5
    // ////////////////////////
    let mut discv5: Discv5 = Discv5::new(enr.clone(), enr_key, config)?;
    discv5.start().await.expect("Start Discovery v5 server");

    client
        .signal_and_wait(
            STATE_DISCV5_STARTED,
            client.run_parameters().test_instance_count,
        )
        .await?;

    // Wait until the mock send a random packet.
    client
        .signal_and_wait(
            STATE_SENT_RANDOM_PACKET,
            client.run_parameters().test_instance_count,
        )
        .await?;

    // Sent requests in parallel.
    let mut handles = vec![];
    for _ in 0..2 {
        let fut = discv5.find_node_designated_peer(another_instance_info.enr.clone(), vec![0]);
        handles.push(tokio::spawn(fut));
    }

    for h in handles {
        if let Err(e) = h.await.unwrap() {
            error!("FINDNODE request failed: {e}");
        }
    }

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    client.record_success().await?;
    Ok(())
}

async fn run_mock(
    client: Client,
    enr: Enr,
    enr_key: CombinedKey,
    config: discv5::Config,
    another_instance_info: InstanceInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    // ////////////////////////
    // Start mock
    // ////////////////////////
    let mut behaviours = VecDeque::new();
    behaviours.push_back(Behaviour {
        expect: Expect::WhoAreYou,
        actions: vec![Action::Ignore(
            "Ingore WHOAREYOU packet to make happen a challenge timeout on Node1 side.".to_string(),
        )],
    });
    behaviours.push_back(Behaviour {
        expect: Expect::MessageWithoutSession,
        actions: vec![Action::SendWhoAreYou],
    });
    behaviours.push_back(Behaviour {
        expect: Expect::MessageWithoutSession,
        actions: vec![Action::Ignore("WHOAREYOU packet already sent.".to_string())],
    });
    behaviours.push_back(Behaviour {
        expect: Expect::Handshake(Request::FindNode),
        actions: vec![Action::EstablishSession, Action::Ignore("todo".to_string())],
    });
    // TODO: handle PING request
    let mut mock = Mock::start(enr, enr_key, config, Behaviours::Sequential(behaviours)).await;

    client
        .signal_and_wait(
            STATE_DISCV5_STARTED,
            client.run_parameters().test_instance_count,
        )
        .await?;

    // Send a random packet.
    // The receiver of the packet reply with WHOAREYOU packet but this mock drops it without replying.
    if let Err(e) = mock.send_random_packet(another_instance_info.enr) {
        error!("Failed to send random packet: {e}");
    }
    tokio::time::sleep(Duration::from_secs(1)).await;

    client
        .signal_and_wait(
            STATE_SENT_RANDOM_PACKET,
            client.run_parameters().test_instance_count,
        )
        .await?;

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    client.record_success().await?;
    Ok(())
}
