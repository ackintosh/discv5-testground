use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::{Discv5, Discv5ConfigBuilder, Enr, ListenConfig};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use std::time::Duration;
use testground::client::Client;
use tracing::error;

const STATE_CONNECTED: &str = "state_connected";
const STATE_COMPLETED: &str = "state_completed";

// Session timeout for Node2 (in second).
const SESSION_TIMEOUT_NODE2: u64 = 5;

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

    // ////////////////////////
    // Start discv5
    // ////////////////////////
    let listen_config = ListenConfig::Ipv4 {
        ip: Ipv4Addr::UNSPECIFIED,
        port: 9000,
    };

    let config = if client.global_seq() == 2 {
        Discv5ConfigBuilder::new(listen_config)
            .session_timeout(Duration::from_secs(SESSION_TIMEOUT_NODE2))
            .build()
    } else {
        Discv5ConfigBuilder::new(listen_config).build()
    };

    let mut discv5: Discv5 = Discv5::new(enr.clone(), enr_key, config)?;
    discv5.start().await.expect("Start Discovery v5 server");

    // //////////////////////////////////////////////////////////////
    // Collect information of all participants in the test case
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo {
        seq: client.global_seq(),
        enr,
    };
    client.record_message(format!(
        "seq: {}, node_id: {}, ip: {}",
        instance_info.seq,
        instance_info.enr.node_id(),
        ip
    ));

    let participants = publish_and_collect(&client, instance_info.clone()).await?;

    // //////////////////////////////////////////////////////////////
    // Construct topology
    // //////////////////////////////////////////////////////////////
    // Run FINDNODE query to connect to other participants.
    if instance_info.seq == 1 {
        for p in participants
            .iter()
            .filter(|&p| p.seq != client.global_seq())
        {
            let _ = discv5
                .find_node_designated_peer(p.enr.clone(), vec![0])
                .await;
        }
    }

    client
        .signal_and_wait(STATE_CONNECTED, run_parameters.test_instance_count)
        .await?;

    client.record_message(format!(
        "peers: {:?}",
        discv5
            .kbuckets()
            .iter()
            .map(|b| (
                b.node.value.ip4().unwrap(),
                b.status.direction,
                b.status.state
            ))
            .collect::<Vec<_>>()
    ));

    // //////////////////////////////////////////////////////////////
    // Send requests in parallel
    // //////////////////////////////////////////////////////////////
    // Wait for the Node2 session to expire.
    tokio::time::sleep(Duration::from_secs(SESSION_TIMEOUT_NODE2 + 2)).await;

    // Send requests in parallel from Node1 to Node2.
    let mut succeeded = true;
    if instance_info.seq == 1 {
        for p in participants
            .iter()
            .filter(|&p| p.seq != client.global_seq())
        {
            let mut handles = vec![];
            for _ in 0..2 {
                let fut = discv5.find_node_designated_peer(p.enr.clone(), vec![0]);
                handles.push(tokio::spawn(fut));
            }

            for h in handles {
                if let Err(e) = h.await.unwrap() {
                    error!("FINDNODE request failed: {e}");
                    succeeded = false;
                }
            }
        }
    }

    tokio::time::sleep(Duration::from_secs(3)).await;

    client
        .signal_and_wait(STATE_COMPLETED, run_parameters.test_instance_count)
        .await?;

    if succeeded {
        client.record_success().await?;
    } else {
        client
            .record_failure(
                "The requests have resulted in failure. Please check the log for details.",
            )
            .await?
    }
    Ok(())
}
