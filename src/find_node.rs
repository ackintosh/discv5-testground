use crate::{collect_instance_info, InstanceInfo};
use chrono::Local;
use discv5::enr::{CombinedKey, EnrBuilder, NodeId};
use discv5::{Discv5, Discv5Config, Key};
use std::net::SocketAddr;
use testground::client::Client;
use testground::{RunParameters, WriteQuery};
use tokio::task;
use tracing::{debug, error, info};

const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "state_completed_to_collect_instance_information";
const STATE_COMPLETED_TO_BUILD_TOPOLOGY: &str = "state_completed_to_build_topology";
const STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY: &str = "state_completed_to_run_find_node_query";

pub(super) async fn find_node(
    client: Client,
    run_parameters: RunParameters,
) -> Result<(), Box<dyn std::error::Error>> {
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
    // SEE: https://github.com/ackintosh/discv5-testground/pull/13#issuecomment-1120430861
    let mut discv5 = Discv5::new(enr, enr_key, Discv5Config::default())?;
    discv5
        .start("0.0.0.0:9000".parse::<SocketAddr>()?)
        .await
        .expect("Start Discovery v5 server");

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

    let other_instances = collect_instance_info(&client, &run_parameters, &instance_info).await?;
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
                metrics.unsolicited_requests_per_second as f64,
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
