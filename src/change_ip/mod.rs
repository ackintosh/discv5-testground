mod params;

use crate::change_ip::params::Params;
use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::{Discv5, Discv5ConfigBuilder, Enr, ListenConfig};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use testground::client::Client;
use testground::network_conf::{
    FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEFAULT_DATA_NETWORK,
};

const STATE_COMPLETED_TO_CONNECT: &str = "state_completed_to_connect";

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
    let params = Params::new(&run_parameters.test_instance_params)?;

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
    let config = Discv5ConfigBuilder::new(listen_config)
        .vote_duration(Duration::from_secs(params.vote_duration))
        .ping_interval(Duration::from_secs(params.ping_interval))
        .enr_peer_update_min(run_parameters.test_instance_count as usize - 1)
        .build();
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
        .signal_and_wait(
            STATE_COMPLETED_TO_CONNECT,
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

    // //////////////////////////////////////////////////////////////
    // Change IP address
    // //////////////////////////////////////////////////////////////
    tokio::time::sleep(Duration::from_secs(params.duration_before)).await;

    if instance_info.seq == 1 {
        let new_ip = change_ip(&client, &participants).await?;
        client.record_message(format!(
            "IP address has been changed from {} to {}.",
            ip, new_ip
        ));
    }

    tokio::time::sleep(Duration::from_secs(params.duration_after)).await;

    if instance_info.seq == 1 {
        println!("debugggg: {:?}", discv5.table_entries());
    }

    client.record_success().await?;
    Ok(())
}

async fn change_ip(
    client: &Client,
    participants: &[InstanceInfo],
) -> Result<IpAddr, Box<dyn std::error::Error>> {
    let participants_ip4 = participants
        .iter()
        .map(|p| p.enr.ip4().expect("ip4"))
        .collect::<Vec<_>>();

    let subnet = client.run_parameters().test_subnet;

    let new_ip = {
        let mut iter = subnet.iter();
        // Calling `next()` twice in order to skip network address and the first one. The first one is reserved by Testground.
        iter.next();
        iter.next();

        iter.find(|i| {
            if let IpAddr::V4(ipv4) = i {
                !participants_ip4.contains(ipv4)
            } else {
                false
            }
        })
        .expect("unused ip")
    };

    client
        .configure_network(NetworkConfiguration {
            network: DEFAULT_DATA_NETWORK.to_owned(),
            ipv4: Some(format!("{}/{}", new_ip, subnet.prefix()).parse().unwrap()),
            ipv6: None,
            enable: true,
            default: LinkShape {
                latency: client
                    .run_parameters()
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
            callback_state: "change_ip".to_owned(),
            callback_target: Some(1),
            routing_policy: RoutingPolicyType::DenyAll,
        })
        .await?;

    Ok(new_ip)
}
