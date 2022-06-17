mod eclipse;
mod find_node;

use serde::de::DeserializeOwned;
use serde::Serialize;
use testground::client::Client;
use testground::network_conf::{
    FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEAFULT_DATA_NETWORK,
};
use testground::RunParameters;
use tokio_stream::StreamExt;

const STATE_NETWORK_CONFIGURED: &str = "state_network_configured";

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

    // //////////////////////////////////////////////////////////////
    // Run test case
    // //////////////////////////////////////////////////////////////
    match run_parameters.test_case.clone().as_str() {
        "find-node" => find_node::find_node(client.clone(), run_parameters.clone()).await?,
        "eclipse-attack-monopolizing-by-incoming-nodes" => {
            eclipse::MonopolizingByIncomingNodes::new(run_parameters.clone())
                .run(client.clone())
                .await?
        }
        _ => unreachable!(),
    };

    Ok(())
}

// Returns the sequence number of this test instance within the test.
async fn get_instance_seq(client: &Client) -> Result<u64, testground::errors::Error> {
    client.signal("get_instance_seq").await
}

async fn get_group_seq(
    client: &Client,
    group_id: &String,
) -> Result<u64, testground::errors::Error> {
    client.signal(format!("get_group_seq_{}", group_id)).await
}

async fn publish_and_collect<T: Serialize + DeserializeOwned>(
    client: &Client,
    run_parameters: &RunParameters,
    info: T,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    const TOPIC: &str = "publish_and_collect";

    client.publish(TOPIC, serde_json::to_string(&info)?).await?;

    let mut stream = client.subscribe(TOPIC).await;

    let mut vec: Vec<T> = vec![];

    for _ in 0..run_parameters.test_instance_count {
        match stream.next().await {
            Some(Ok(other)) => {
                let info: T = serde_json::from_str(&other)?;
                vec.push(info);
            }
            Some(Err(e)) => return Err(Box::new(e)),
            None => unreachable!(),
        }
    }

    Ok(vec)
}
