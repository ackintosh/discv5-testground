use crate::concurrent_requests::InstanceInfo;
use crate::utils::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder};
use discv5::{Discv5, ListenConfig};
use std::net::Ipv4Addr;
use std::time::Duration;
use testground::client::Client;
use tracing::{error, info};

const STATE_DISCV5_STARTED: &str = "state_discv5_started";
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

    match client.global_seq() {
        1 => {
            // Sent requests in parallel.
            let mut handles = vec![];
            for i in 0..2 {
                let fut = discv5.talk_req(another_instance_info.enr.clone(), vec![0], vec![i]);
                handles.push(tokio::spawn(fut));
            }

            for h in handles {
                match h.await.unwrap() {
                    Ok(res) => info!("Response: {:?}", res),
                    Err(e) => error!("Request failed: {e}"),
                }
            }
        }
        2 => {
            let mut req_count = 0;
            let mut event = discv5.event_stream().await.expect("event stream");
            while let Some(ev) = event.recv().await {
                match ev {
                    discv5::Event::Discovered(_) => {}
                    discv5::Event::EnrAdded { .. } => {}
                    discv5::Event::NodeInserted { .. } => {}
                    discv5::Event::SessionEstablished(_, _) => {}
                    discv5::Event::SocketUpdated(_) => {}
                    discv5::Event::TalkRequest(req) => {
                        req_count += 1;
                        info!("TalkRequest: {:?}", req);
                        let response = req.body().to_vec();
                        if let Err(e) = req.respond(response) {
                            error!("Failed to send response: {:?}", e);
                        }

                        if req_count == 2 {
                            break;
                        }
                    }
                }
            }
        }
        _ => unreachable!(),
    }

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    client.record_success().await?;
    Ok(())
}
