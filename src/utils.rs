use serde::de::DeserializeOwned;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use testground::client::Client;
use tokio_stream::StreamExt;

pub(crate) async fn publish_and_collect<T: Serialize + DeserializeOwned>(
    client: &Client,
    info: T,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    const TOPIC: &str = "publish_and_collect";

    client
        .publish(TOPIC, Cow::Owned(serde_json::to_value(&info)?))
        .await?;

    let mut stream = client.subscribe(TOPIC, u16::MAX.into()).await;

    let mut vec: Vec<T> = vec![];

    for _ in 0..client.run_parameters().test_instance_count {
        match stream.next().await {
            Some(Ok(other)) => {
                let info: T = serde_json::from_value(other)?;
                vec.push(info);
            }
            Some(Err(e)) => return Err(Box::new(e)),
            None => unreachable!(),
        }
    }

    Ok(vec)
}

pub(crate) fn get_param<T: FromStr>(
    k: &str,
    instance_params: &HashMap<String, String>,
) -> Result<T, String> {
    instance_params
        .get(k)
        .ok_or(format!("{k} is not specified."))?
        .parse::<T>()
        .map_err(|_| format!("Failed to parse instance_param. key: {}", k))
}
