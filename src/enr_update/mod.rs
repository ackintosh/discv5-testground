mod params;

use crate::enr_update::params::Params;
use crate::utils::publish_and_collect;
use chrono::Local;
use discv5::enr::CombinedKey;
use discv5::{Discv5, Enr, ListenConfig};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use testground::client::Client;
use tokio::{sync, task};
use tracing::{debug, error, info};

const STATE_COMPLETED_ESTABLISH_CONNECTIONS: &str = "state_completed_establish_connections";
const STATE_COMPLETED: &str = "state_completed";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr,
}

pub(super) async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let run_parameters = client.run_parameters();
    let params = Params::new(&run_parameters.test_instance_params)?;

    // ////////////////////////
    // Construct a local Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();

    let enr = if client.global_seq() == 1 {
        Enr::builder().build(&enr_key).expect("Construct an Enr")
    } else {
        Enr::builder()
            .ip(run_parameters
                .data_network_ip()?
                .expect("IP address for the data network"))
            .udp4(9000)
            .build(&enr_key)
            .expect("Construct an Enr")
    };

    info!("ENR: {:?}", enr);
    info!("NodeId: {}", enr.node_id());

    // //////////////////////////////////////////////////////////////
    // Start Discovery v5 server
    // //////////////////////////////////////////////////////////////
    let mut discv5: Discv5 = Discv5::new(
        enr,
        enr_key,
        discv5::ConfigBuilder::new(ListenConfig::default())
            .ping_interval(Duration::from_secs(params.ping_interval))
            .build(),
    )?;
    discv5.start().await.expect("Start Discovery v5 server");
    let started_up_at = Local::now();

    // //////////////////////////////////////////////////////////////
    // Collect information of all participants in the test case
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo {
        seq: client.global_seq(),
        enr: discv5.local_enr(),
    };
    debug!("instance_info: {:?}", instance_info);

    let participants = publish_and_collect(&client, instance_info.clone()).await?;

    let maybe_receiver = if instance_info.seq == 1 {
        let (sender, receiver) = sync::oneshot::channel();
        let mut event_stream = discv5.event_stream().await.expect("Discv5Event");

        task::spawn(async move {
            while let Some(event) = event_stream.recv().await {
                if let discv5::Event::SocketUpdated(socket_addr) = event {
                    sender.send(socket_addr).unwrap();
                    break;
                }
            }
        });

        Some(receiver)
    } else {
        None
    };

    // //////////////////////////////////////////////////////////////
    // Establish connections
    // //////////////////////////////////////////////////////////////
    if instance_info.seq == 1 {
        for p in participants
            .iter()
            .filter(|&p| p.seq != client.global_seq())
        {
            if let Err(e) = discv5
                .find_node_designated_peer(p.enr.clone(), vec![0])
                .await
            {
                error!("Failed to run FIND_NODE query: {e}");
            }
        }
    }

    client
        .signal_and_wait(
            STATE_COMPLETED_ESTABLISH_CONNECTIONS,
            run_parameters.test_instance_count,
        )
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

    if let Some(receiver) = maybe_receiver {
        match receiver.await {
            Ok(socket_addr) => {
                info!("Discv5Event::SocketUpdated {socket_addr}");
                client.record_message(format!(
                    "The socket has been updated {} seconds after startup.",
                    (Local::now() - started_up_at).num_seconds()
                ));
            }
            Err(e) => {
                error!("RecvError: {e}");
            }
        }
    }

    client
        .signal_and_wait(STATE_COMPLETED, run_parameters.test_instance_count)
        .await?;

    client.record_success().await?;
    Ok(())
}
