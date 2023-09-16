mod concurrent_requests;
mod eclipse;
mod enr_update;
mod find_node;
mod ip_change;
mod mock;
mod sandbox;
mod utils;

use testground::client::Client;
use testground::network_conf::{
    FilterAction, LinkShape, NetworkConfiguration, RoutingPolicyType, DEFAULT_DATA_NETWORK,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new_and_init().await?;

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
    client
        .configure_network(NetworkConfiguration {
            network: DEFAULT_DATA_NETWORK.to_owned(),
            ipv4: None,
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
            callback_state: "state_network_configured".to_owned(),
            callback_target: None,
            routing_policy: RoutingPolicyType::DenyAll,
        })
        .await?;

    // //////////////////////////////////////////////////////////////
    // Run test case
    // //////////////////////////////////////////////////////////////
    match client.run_parameters().test_case.clone().as_str() {
        "find-node" => find_node::run(client.clone()).await?,
        "concurrent-requests" => concurrent_requests::run(client).await?,
        "concurrent-requests_whoareyou-timeout" => {
            concurrent_requests::whoareyou_timeout::run(client).await?
        }
        "concurrent-requests_before-establishing-session" => {
            concurrent_requests::before_establishing_session::run(client).await?
        }
        "eclipse-attack-monopolizing-by-incoming-nodes" => {
            eclipse::MonopolizingByIncomingNodes::new()
                .run(client.clone())
                .await?
        }
        "enr-update" => enr_update::run(client.clone()).await?,
        "ip-change" => ip_change::run(client).await?,
        "sandbox" => sandbox::run(client).await?,
        _ => unreachable!(),
    };

    Ok(())
}
