use crate::mock::{Action, Behaviour, Expect, Mock};
use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::{Discv5, Enr, ListenConfig};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::Duration;
use testground::client::Client;
use tracing::error;

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
        .request_timeout(Duration::from_secs(3))
        .build();

    match client.global_seq() {
        1 => run_discv5(client, enr, enr_key, config, another_instance_info).await?,
        2 => run_mock(client, enr, enr_key, config).await?,
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

    let result = discv5
        .find_node_designated_peer(another_instance_info.enr.clone(), vec![0])
        .await;

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    println!("result: {result:?}");
    if result.is_err() {
        client.record_success().await?;
    } else {
        client.record_failure("the request succeeded.")
    }
    Ok(())
}

async fn run_mock(
    client: Client,
    enr: Enr,
    enr_key: CombinedKey,
    config: discv5::Config,
) -> Result<(), Box<dyn std::error::Error>> {
    // ////////////////////////
    // Start mock
    // ////////////////////////
    let mut behaviours = VecDeque::new();
    behaviours.push_back(Behaviour {
        expect: Expect::MessageWithoutSession,
        action: Action::Ignore("Ignoring a message".to_string()),
    });
    let mut _mock = Mock::start(enr, enr_key, config, behaviours).await;

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
