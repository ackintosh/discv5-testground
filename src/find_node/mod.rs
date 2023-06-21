use crate::utils::publish_and_collect;
use chrono::Local;
use discv5::enr::{CombinedKey, EnrBuilder, NodeId};
use discv5::{Discv5, Discv5ConfigBuilder, Enr, Key, ListenConfig};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use testground::client::Client;
use testground::WriteQuery;
use tokio::task;
use tracing::{debug, error, info};

const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "state_completed_to_collect_instance_information";
const STATE_COMPLETED_TO_BUILD_TOPOLOGY: &str = "state_completed_to_build_topology";
const STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY: &str = "state_completed_to_run_find_node_query";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr,
    is_bootstrap_node: bool,
}

impl InstanceInfo {
    async fn new(client: &Client, enr: Enr) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = client.global_seq();

        // NOTE: For now, #1 is bootstrap node.
        let is_bootstrap_node = seq == 1;

        Ok(InstanceInfo {
            seq,
            enr,
            is_bootstrap_node,
        })
    }
}

pub(super) async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let run_parameters = client.run_parameters();
    // ////////////////////////
    // Construct a local Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();
    let enr = EnrBuilder::new("v4")
        .ip(run_parameters
            .data_network_ip()?
            .expect("IP address for the data network"))
        .udp4(9000)
        .build(&enr_key)
        .expect("Construct an Enr");

    info!("ENR: {:?}", enr);
    info!("NodeId: {}", enr.node_id());

    // //////////////////////////////////////////////////////////////
    // Start Discovery v5 server
    // //////////////////////////////////////////////////////////////
    let listen_config = ListenConfig::Ipv4 {
        ip: Ipv4Addr::UNSPECIFIED,
        port: 9000,
    };
    let mut discv5: Discv5 = Discv5::new(
        enr,
        enr_key,
        Discv5ConfigBuilder::new(listen_config).build(),
    )?;
    discv5.start().await.expect("Start Discovery v5 server");

    // Observe Discv5 events.
    let mut event_stream = discv5.event_stream().await.expect("Discv5Event");
    task::spawn(async move {
        while let Some(event) = event_stream.recv().await {
            info!("Discv5Event: {:?}", event);
        }
    });

    // //////////////////////////////////////////////////////////////
    // Collect information of all participants in the test case
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo::new(&client, discv5.local_enr()).await?;
    debug!("instance_info: {:?}", instance_info);

    let other_instances = collect_other_instance_info(&client, &instance_info).await?;
    debug!("other_instances: {:?}", other_instances);

    client
        .signal_and_wait(
            STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION,
            run_parameters.test_instance_count,
        )
        .await?;

    // //////////////////////////////////////////////////////////////
    // Star topology
    // //////////////////////////////////////////////////////////////
    // NOTE: Assumes only 1 bootstrap node.
    let key: Key<NodeId> = discv5.local_enr().node_id().into();
    if instance_info.is_bootstrap_node {
        for i in other_instances.iter() {
            discv5.add_enr(i.enr.clone())?;
        }
    } else {
        let bootstrap_node = other_instances
            .iter()
            .find(|&i| i.is_bootstrap_node)
            .expect("Bootstrap node");

        // Emit distance to the bootstrap node.
        let bootstrap_key: Key<NodeId> = bootstrap_node.enr.node_id().into();
        info!(
            "Distance between `self` and `bootstrap`: {}",
            key.log2_distance(&bootstrap_key).expect("Distance")
        );

        discv5.add_enr(bootstrap_node.enr.clone())?;
    }

    client
        .signal_and_wait(
            STATE_COMPLETED_TO_BUILD_TOPOLOGY,
            run_parameters.test_instance_count,
        )
        .await?;

    if instance_info.is_bootstrap_node {
        let buckets = discv5.kbuckets();
        for b in buckets.buckets_iter() {
            for n in b.iter() {
                info!("Node: node_id: {}, enr: {}", n.key.preimage(), n.value);
            }
        }
    }

    // //////////////////////////////////////////////////////////////
    // Run FINDNODE query
    // //////////////////////////////////////////////////////////////
    let mut failed = false;

    if instance_info.is_bootstrap_node {
        println!("Skipped to run FINDNODE query because this is the bootstrap node.");
    } else {
        for target in other_instances {
            if target.is_bootstrap_node {
                continue;
            }

            // Emit distance to the target.
            let target_key: Key<NodeId> = target.enr.node_id().into();
            info!("target: {}", target.enr.node_id());
            info!(
                "Distance between `self` and `target`: {}",
                key.log2_distance(&target_key).expect("Distance")
            );

            if let Some(enr) = discv5.find_enr(&target.enr.node_id()) {
                info!(
                    "The target is already exists in the routing table. ENR: {:?}",
                    enr
                );
            } else {
                let enrs = discv5
                    .find_node(target.enr.node_id())
                    .await
                    .expect("FINDNODE query");

                if enrs.is_empty() {
                    error!("Found no ENRs");
                    failed = true;
                } else {
                    info!("Found ENRs: {:?}", enrs);

                    // The target node should be found because the bootstrap node knows all the nodes in our star topology.
                    if enrs.iter().any(|enr| enr.node_id() == target.enr.node_id()) {
                        info!("Found the target");
                    } else {
                        error!(
                            "Couldn't find the target. node_id: {}",
                            target.enr.node_id()
                        );
                        failed = true;
                    }
                }
            }

            // //////////////////////////////////////////////////////////////
            // Record metrics
            // //////////////////////////////////////////////////////////////
            let metrics = discv5.metrics();
            let write_query = WriteQuery::new(
                Local::now().into(),
                format!(
                    "discv5-testground_{}_{}",
                    run_parameters.test_case, run_parameters.test_run
                ),
            )
            .add_field("active_sessions", metrics.active_sessions as u64)
            .add_field(
                "unsolicited_requests_per_second",
                metrics.unsolicited_requests_per_second,
            )
            .add_field("bytes_sent", metrics.bytes_sent as u64)
            .add_field("bytes_recv", metrics.bytes_recv as u64)
            .add_tag("instance_seq", instance_info.seq);
            client.record_metric(write_query).await?;
        }
    }

    client
        .signal_and_wait(
            STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY,
            run_parameters.test_instance_count,
        )
        .await?;

    // //////////////////////////////////////////////////////////////
    // Record result of this test
    // //////////////////////////////////////////////////////////////
    if failed {
        client
            .record_failure("Failures have happened, please check error logs for details.")
            .await?;
    } else {
        client.record_success().await?;
    }

    Ok(())
}

async fn collect_other_instance_info(
    client: &Client,
    instance_info: &InstanceInfo,
) -> Result<Vec<InstanceInfo>, Box<dyn std::error::Error>> {
    let mut info = publish_and_collect(client, instance_info.clone()).await?;

    if let Some(pos) = info.iter().position(|i| i.seq == instance_info.seq) {
        info.remove(pos);
    }

    Ok(info)
}
