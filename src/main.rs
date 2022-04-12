use discv5::enr::{CombinedKey, Enr, EnrBuilder};
use discv5::{Discv5, Discv5Config};
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use testground::client::Client;
use testground::RunParameters;
use tokio_stream::StreamExt;

const TOPIC_INSTANCE_INFORMATION: &str = "topic_instance_information";
const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "state_completed_to_collect_instance_information";
const STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY: &str = "state_completed_to_run_find_node_query";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, run_parameters) = testground::client::Client::new().await?;

    client.record_message(format!(
        "plan: {}, case: {}, run: {}, group_id: {}",
        run_parameters.test_plan,
        run_parameters.test_case,
        run_parameters.test_run,
        run_parameters.test_group_id
    )); // Debug

    // ////////////////////////
    // Generate Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();
    let enr = EnrBuilder::new("v4")
        .ip(get_subnet_addr(&run_parameters.test_subnet)?)
        .udp(9000)
        .build(&enr_key)
        .expect("Construct an Enr");

    // //////////////////////////////////////////////////////////////
    // Start Discovery v5 server
    // //////////////////////////////////////////////////////////////
    let mut discv5 = Discv5::new(enr, enr_key, Discv5Config::default())?;
    discv5
        .start("0.0.0.0:9000".parse::<SocketAddr>()?)
        .await
        .expect("Start Discovery v5 server");

    // //////////////////////////////////////////////////////////////
    // Collect information of all instances within the test
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo::new(&client, &run_parameters, discv5.local_enr()).await?;
    client.record_message(format!("Debug: instance_info = {:?}", instance_info)); // Debug

    let other_instances =
        collect_instance_information(&client, &run_parameters, &instance_info).await?;
    client.record_message(format!("{:?}", other_instances)); // Debug

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
    if instance_info.is_bootstrap_node {
        for i in other_instances.iter() {
            discv5.add_enr(i.enr.clone())?;
        }
    } else {
        let bootstrap_node = other_instances
            .iter()
            .find(|&i| i.is_bootstrap_node)
            .expect("Bootstrap node");

        discv5.add_enr(bootstrap_node.enr.clone())?;
        client.record_message(format!("bootstrap_node: {:?}", bootstrap_node)); // Debug
    }

    // //////////////////////////////////////////////////////////////
    // Run FIND_NODE query
    // //////////////////////////////////////////////////////////////
    // NOTE: Assumes only 1 target node.
    if instance_info.is_target_node {
        client.record_message("Skipped to run FIND_NODE query because this is the target_node.");
    } else {
        let target_node = other_instances
            .iter()
            .find(|&i| i.is_target_node)
            .expect("Target node");
        let enrs = discv5
            .find_node(target_node.enr.node_id())
            .await
            .expect("FIND_NODE query");
        client.record_message(format!("Enrs: {:?}", enrs));
    }

    client
        .signal_and_wait(
            STATE_COMPLETED_TO_RUN_FIND_NODE_QUERY,
            run_parameters.test_instance_count,
        )
        .await?;

    // //////////////////////////////////////////////////////////////
    // Shutdown Discovery v5 server
    // //////////////////////////////////////////////////////////////
    client.record_success().await?;
    discv5.shutdown();

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr<CombinedKey>,
    is_bootstrap_node: bool,
    is_target_node: bool,
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
        // NOTE: For now, the instance with the last sequence number is target_node.
        let is_target_node = seq == run_parameters.test_instance_count;

        Ok(InstanceInfo {
            seq,
            enr,
            is_bootstrap_node,
            is_target_node,
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
