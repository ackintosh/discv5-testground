use crate::utils::publish_and_collect;
use discv5::enr::CombinedKey;
use discv5::{Discv5, Enr, Event, ListenConfig};
use serde::{Deserialize, Serialize};
use testground::client::Client;
use tracing::{debug, info};

const STATE_READY_TO_START_SIM: &str = "state_ready_to_start_sim";
const STATE_FINISHED: &str = "state_finished";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceInfo {
    // The sequence number of this test instance within the test.
    seq: u64,
    enr: Enr,
}

pub(super) async fn run(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let run_parameters = client.run_parameters();

    assert_eq!(run_parameters.test_instance_count, 2);

    // ////////////////////////
    // Construct local Enr
    // ////////////////////////
    let enr_key = CombinedKey::generate_secp256k1();
    let enr = Enr::builder()
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
    let mut discv5: Discv5 = Discv5::new(
        enr,
        enr_key,
        discv5::ConfigBuilder::new(ListenConfig::default()).build(),
    )?;
    discv5.start().await.expect("Start Discovery v5 server");

    // //////////////////////////////////////////////////////////////
    // Collect information of all participants in the test case
    // //////////////////////////////////////////////////////////////
    let instance_info = InstanceInfo {
        seq: client.global_seq(),
        enr: discv5.local_enr(),
    };
    debug!("instance_info: {:?}", instance_info);

    let another_node = publish_and_collect(&client, instance_info.clone())
        .await?
        .into_iter()
        .find(|info| info.seq != client.global_seq())
        .unwrap();

    client
        .signal_and_wait(STATE_READY_TO_START_SIM, run_parameters.test_instance_count)
        .await?;

    let protocol: Vec<u8> = vec![1];
    let request: Vec<u8> = vec![1, 2, 3];
    let response: Vec<u8> = vec![4, 5, 6];

    let test_result = match client.global_seq() {
        1 => {
            // Send TALKREQ
            match discv5.talk_req(another_node.enr, protocol, request).await {
                Ok(talk_response) => {
                    if talk_response == response {
                        Ok(())
                    } else {
                        Err(format!(
                            "Invalid response. expected: {response:?}, actual: {talk_response:?}"
                        ))
                    }
                }
                Err(e) => Err(e.to_string()),
            }
        }
        2 => {
            // Respond TALKREQ
            let mut event_stream = discv5.event_stream().await.unwrap();
            let mut result = Ok(());
            while let Some(event) = event_stream.recv().await {
                match event {
                    Event::TalkRequest(talk_request) => {
                        if talk_request.protocol() == &protocol && talk_request.body() == &request {
                            if let Err(e) = talk_request.respond(response.clone()) {
                                result = Err(e.to_string());
                            }
                        } else {
                            result = Err(format!(
                                "Invalid request. expected: {request:?}, actual: {talk_request:?}"
                            ));
                        }
                        break;
                    }
                    _ => continue,
                }
            }
            result
        }
        _ => unreachable!(),
    };

    client
        .signal_and_wait(STATE_FINISHED, client.run_parameters().test_instance_count)
        .await?;

    if let Err(e) = test_result {
        client.record_failure(e).await.unwrap();
    } else {
        client.record_success().await?;
    }

    Ok(())
}
