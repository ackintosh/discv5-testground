use discv5::enr;
use discv5::enr::CombinedKey;
use discv5::enr::k256::elliptic_curve::rand_core::RngCore;
use testground::client::Client;
use testground::RunParameters;

pub(super) async fn monopolizing_connections(
    client: Client,
    run_parameters: RunParameters,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("monopolizing_connections: {:?}", run_parameters);

    client.record_success().await?;
    Ok(())
}