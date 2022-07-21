use crate::eclipse::generate_deterministic_keypair;
use crate::publish_and_collect;
use discv5::enr::{CombinedKey, EnrBuilder, NodeId};
use discv5::{ConnectionDirection, Discv5, Discv5ConfigBuilder, Enr};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use testground::client::Client;
use tokio::task;
use tracing::debug;

const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION";
const STATE_ATTACKERS_SENT_QUERY: &str = "STATE_ATTACKERS_SENT_QUERY";
const STATE_DONE: &str = "STATE_DONE";

pub(crate) struct PretendingNotToKnow {}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Role {
    Victim,
    Attacker,
}

impl From<&str> for Role {
    fn from(test_group_id: &str) -> Self {
        match test_group_id {
            "victim" => Role::Victim,
            "attackers" => Role::Attacker,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstanceInfo {
    enr: Enr,
    role: Role,
}

impl PretendingNotToKnow {
    pub(crate) async fn run(&self, client: Client) -> Result<(), Box<dyn std::error::Error>> {
        // Note: The seq starts from 1.
        let role: Role = client.run_parameters().test_group_id.as_str().into();
        client.record_message(format!(
            "role: {:?}, group_seq: {}",
            role,
            client.group_seq()
        ));

        // ////////////////////////
        // Construct a local Enr
        // ////////////////////////
        let enr_key = Self::generate_deterministic_keypair(client.group_seq(), &role);
        let enr = EnrBuilder::new("v4")
            .ip(client
                .run_parameters()
                .data_network_ip()?
                .expect("IP address for the data network"))
            .udp4(9000)
            .build(&enr_key)
            .expect("Construct an Enr");

        // //////////////////////////////////////////////////////////////
        // Start Discovery v5 server
        // //////////////////////////////////////////////////////////////
        let discv5_config = Discv5ConfigBuilder::new()
            .incoming_bucket_limit(8)
            .session_timeout(match role {
                Role::Victim => Duration::from_secs(100),
                Role::Attacker => Duration::from_secs(0),
            })
            .build();
        let mut discv5 = Discv5::new(enr, enr_key, discv5_config)?;
        discv5
            .start("0.0.0.0:9000".parse::<SocketAddr>()?)
            .await
            .expect("Start Discovery v5 server");

        // Observe Discv5 events.
        let mut event_stream = discv5.event_stream().await.expect("Discv5Event");
        task::spawn(async move {
            while let Some(event) = event_stream.recv().await {
                debug!("Discv5Event: {:?}", event);
            }
        });

        // //////////////////////////////////////////////////////////////
        // Collect information of all participants in the test case
        // //////////////////////////////////////////////////////////////
        let instance_info = InstanceInfo {
            enr: discv5.local_enr(),
            role,
        };

        let (victim, attackers) = self.collect_instance_info(&client, &instance_info).await?;

        client
            .signal_and_wait(
                STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION,
                client.run_parameters().test_instance_count,
            )
            .await?;

        // //////////////////////////////////////////////////////////////
        // Play the role
        // //////////////////////////////////////////////////////////////
        match instance_info.role {
            Role::Victim => self.play_victim(discv5, client, &attackers).await?,
            Role::Attacker => self.play_attacker(discv5, client, &victim).await?,
        }

        Ok(())
    }

    fn generate_deterministic_keypair(group_seq: u64, role: &Role) -> CombinedKey {
        // Generate 20 key pairs. Distances between the first key pair and all other ones are the
        // same. So in the node with the first key pair, node ids given from the other ones will be
        // inserted into the same bucket.
        //
        // The 20 key pairs generated are assigned to participants according to its role as follows:
        // - 0: victim
        // - 1: attacker
        // - 2: attacker
        // - 3: attacker
        // ...
        // - 19: attacker
        //
        // The `122488` seed is a pre-computed one for this function. See `find_seed_same_bucket()`
        // in https://github.com/sigp/discv5/blob/master/src/discv5/test.rs for more details of the
        // pre-computing.
        let mut keypairs = generate_deterministic_keypair(20, 122488);

        let index = match role {
            Role::Victim => group_seq,
            Role::Attacker => (group_seq + 1), // Take the number of victim into account
        } - 1; // The group_seq starts from 1, not from 0, so we should minus one here.
        keypairs.remove(usize::try_from(index).expect("Valid as usize"))
    }

    async fn collect_instance_info(
        &self,
        client: &Client,
        instance_info: &InstanceInfo,
    ) -> Result<(InstanceInfo, Vec<InstanceInfo>), Box<dyn std::error::Error>> {
        let mut victim = vec![];
        let mut attackers = vec![];

        for i in publish_and_collect(client, instance_info.clone()).await? {
            match i.role {
                Role::Victim => victim.push(i),
                Role::Attacker => attackers.push(i),
            }
        }

        assert!(victim.len() == 1 && attackers.len() == 19);

        Ok((victim.remove(0), attackers))
    }

    async fn play_victim(
        &self,
        discv5: Discv5,
        client: Client,
        attackers: &Vec<InstanceInfo>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        client
            .barrier(
                STATE_ATTACKERS_SENT_QUERY,
                u64::try_from(attackers.len()).unwrap(),
            )
            .await?;

        println!("connected peers: {}", discv5.connected_peers());

        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Err(e) = discv5.find_node(NodeId::random()).await {
                println!("query error: {:?}", e);
            }

            for (i, bucket) in discv5.kbuckets().buckets_iter().enumerate() {
                if bucket.num_entries() == 0 {
                    continue;
                }
                assert_eq!(i, 255);

                let mut incoming = 0;
                let mut outgoing = 0;
                for node in bucket.iter() {
                    match node.status.direction {
                        ConnectionDirection::Incoming => incoming += 1,
                        ConnectionDirection::Outgoing => outgoing += 1,
                    }
                }

                println!(
                    "bucket index:{}: incoming:{}, outgoing:{}",
                    i, incoming, outgoing
                );
            }
        }

        client
            .signal_and_wait(STATE_DONE, client.run_parameters().test_instance_count)
            .await?;

        client.record_success().await?;
        Ok(())
    }

    async fn play_attacker(
        &self,
        discv5: Discv5,
        client: Client,
        victim: &InstanceInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        discv5.add_enr(victim.enr.clone())?;
        if let Err(e) = discv5.find_node(NodeId::random()).await {
            client.record_message(format!("Failed to run query: {}", e));
        }

        // Inform that sending query has been done.
        client.signal(STATE_ATTACKERS_SENT_QUERY).await?;

        // Wait until checking on the victim has been done.
        client
            .signal_and_wait(STATE_DONE, client.run_parameters().test_instance_count)
            .await?;

        client.record_success().await?;
        Ok(())
    }
}
