use chrono::Local;
use discv5::enr::{CombinedKey, Enr, EnrBuilder, NodeId};
use discv5::{Discv5, Discv5Config, Key};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use testground::client::Client;
use testground::network_conf::{
    FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEAFULT_DATA_NETWORK,
};
use testground::{RunParameters, WriteQuery};
use tokio::task;
use tokio_stream::StreamExt;
use tracing::{debug, error, info};

const TOPIC_INSTANCE_INFORMATION: &str = "aaa_topic_instance_information";
const STATE_NETWORK_CONFIGURED: &str = "state_network_configured";
const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "state_completed_to_collect_instance_information";
const STATE_COMPLETED_TO_BUILD_TOPOLOGY: &str = "state_completed_to_build_topology";
const STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY: &str = "state_completed_to_run_find_node_query";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, run_parameters) = testground::client::Client::new().await?;

    // Enable tracing.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
        .expect("EnvFilter");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .try_init();

    // ////////////////////////
    // Configure network
    // ////////////////////////
    client.wait_network_initialized().await?;

    client
        .configure_network(NetworkConfiguration {
            network: DEAFULT_DATA_NETWORK.to_owned(),
            ipv4: None,
            ipv6: None,
            enable: true,
            default: LinkShape {
                latency: run_parameters
                    .test_instance_params
                    .get("latency")
                    .ok_or("latency is not specified")?
                    .parse::<u64>()?
                    * 1_000_000, // Translate from millisecond to nanosecond
                jitter: 0,
                bandwidth: 1048576, // 1Mib
                filter: FilterAction::Accept,
                loss: 0.0,
                corrupt: 0.0,
                corrupt_corr: 0.0,
                reorder: 0.0,
                reorder_corr: 0.0,
                duplicate: 0.0,
                duplicate_corr: 0.0,
            },
            rules: None,
            callback_state: STATE_NETWORK_CONFIGURED.to_owned(),
            callback_target: None,
            routing_policy: RoutingPolicyType::DenyAll,
        })
        .await?;

    client
        .barrier(STATE_NETWORK_CONFIGURED, run_parameters.test_instance_count)
        .await?;

    // ////////////////////////
    // Construct a local Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();
    let enr = EnrBuilder::new("v4")
        .ip(get_subnet_addr(&run_parameters.test_subnet)?)
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
    // Collect information of all instances within the test
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo::new(&client, &run_parameters, discv5.local_enr()).await?;
    debug!("instance_info: {:?}", instance_info);

    let other_instances =
        collect_instance_information(&client, &run_parameters, &instance_info).await?;
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
    // Record result of this test and shutdown Discovery v5 server
    // //////////////////////////////////////////////////////////////
    if failed {
        client
            .record_failure("Failures have happened, please check error logs for details.")
            .await?;
    } else {
        client.record_success().await?;
    }
    discv5.shutdown();

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr<CombinedKey>,
    is_bootstrap_node: bool,
}

impl InstanceInfo {
    async fn new(
        client: &Client,
        run_parameters: &RunParameters,
        enr: Enr<CombinedKey>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = get_instance_seq(client, run_parameters).await?;

        // NOTE: For now, #1 is bootstrap node.
        let is_bootstrap_node = seq == 1;

        Ok(InstanceInfo {
            seq,
            enr,
            is_bootstrap_node,
        })
    }
}

// Returns the sequence number of this test instance within the test.
async fn get_instance_seq(
    client: &Client,
    run_parameters: &RunParameters,
) -> Result<u64, testground::errors::Error> {
    client
        .signal(format!(
            "get_instance_seq:{}",
            run_parameters.test_run.clone()
        ))
        .await
}

fn get_subnet_addr(subnet: &IpNetwork) -> Result<IpAddr, std::io::Error> {
    for interface in if_addrs::get_if_addrs()? {
        let ip = interface.addr.ip();
        if subnet.contains(ip) {
            return Ok(ip);
        }
    }

    panic!("No network interface found."); // TODO: error handling
}

async fn collect_instance_information(
    client: &Client,
    run_parameters: &RunParameters,
    instance_info: &InstanceInfo,
) -> Result<Vec<InstanceInfo>, Box<dyn std::error::Error>> {
    client
        .publish(
            TOPIC_INSTANCE_INFORMATION,
            serde_json::to_string(&instance_info)?,
        )
        .await?;

    let mut stream = client.subscribe(TOPIC_INSTANCE_INFORMATION).await;

    let mut other_instances: Vec<InstanceInfo> = vec![];

    for _ in 0..run_parameters.test_instance_count {
        match stream.next().await {
            Some(Ok(other)) => {
                let other_instance_info: InstanceInfo = serde_json::from_str(&other)?;
                if other_instance_info.seq != instance_info.seq {
                    other_instances.push(other_instance_info);
                }
            }
            Some(Err(e)) => return Err(Box::new(e)),
            None => unreachable!(),
        }
    }

    Ok(other_instances)
}
