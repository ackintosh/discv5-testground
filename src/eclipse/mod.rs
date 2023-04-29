use crate::utils::publish_and_collect;
use discv5::enr::k256::elliptic_curve::rand_core::RngCore;
use discv5::enr::k256::elliptic_curve::rand_core::SeedableRng;
use discv5::enr::{CombinedKey, EnrBuilder, NodeId};
use discv5::{enr, Discv5, Discv5ConfigBuilder, Enr};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::u64;
use testground::client::Client;
use tokio::task;
use tracing::debug;

const STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION: &str =
    "STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION";
const STATE_ATTACKERS_SENT_QUERY: &str = "STATE_ATTACKERS_SENT_QUERY";
const STATE_DONE: &str = "STATE_DONE";

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Role {
    Victim,
    Honest,
    Attacker,
}

impl From<&str> for Role {
    fn from(test_group_id: &str) -> Self {
        match test_group_id {
            "victim" => Role::Victim,
            "honest" => Role::Honest,
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

pub(super) struct MonopolizingByIncomingNodes {}

impl MonopolizingByIncomingNodes {
    pub(super) fn new() -> Self {
        MonopolizingByIncomingNodes {}
    }

    pub(super) async fn run(&self, client: Client) -> Result<(), Box<dyn std::error::Error>> {
        let run_parameters = client.run_parameters();
        // Note: The seq starts from 1.
        let role: Role = run_parameters.test_group_id.as_str().into();
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
            .ip(run_parameters
                .data_network_ip()?
                .expect("IP address for the data network"))
            .udp4(9000)
            .build(&enr_key)
            .expect("Construct an Enr");

        // //////////////////////////////////////////////////////////////
        // Start Discovery v5 server
        // //////////////////////////////////////////////////////////////
        let discv5_config = Discv5ConfigBuilder::new()
            .incoming_bucket_limit(
                run_parameters
                    .test_instance_params
                    .get("incoming_bucket_limit")
                    .expect("incoming_bucket_limit")
                    .parse::<usize>()
                    .expect("Valid as usize"),
            )
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

        let (victim, honest, attackers) =
            self.collect_instance_info(&client, &instance_info).await?;

        client
            .signal_and_wait(
                STATE_COMPLETED_TO_COLLECT_INSTANCE_INFORMATION,
                run_parameters.test_instance_count,
            )
            .await?;

        // //////////////////////////////////////////////////////////////
        // Play the role
        // //////////////////////////////////////////////////////////////
        match instance_info.role {
            Role::Victim => {
                self.play_victim(discv5, client, &honest, &attackers)
                    .await?
            }
            Role::Honest => self.play_honest(client).await?,
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
        // - 1: honest
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
            Role::Honest => group_seq + 1, // Take the number of victim into account
            Role::Attacker => group_seq + 2, // Take the number of victim + honest into account
        } - 1; // The group_seq starts from 1, not from 0, so we should minus one here.
        keypairs.remove(usize::try_from(index).expect("Valid as usize"))
    }

    async fn collect_instance_info(
        &self,
        client: &Client,
        instance_info: &InstanceInfo,
    ) -> Result<(InstanceInfo, InstanceInfo, Vec<InstanceInfo>), Box<dyn std::error::Error>> {
        let mut victim = vec![];
        let mut honest = vec![];
        let mut attackers = vec![];

        for i in publish_and_collect(client, instance_info.clone()).await? {
            match i.role {
                Role::Victim => victim.push(i),
                Role::Honest => honest.push(i),
                Role::Attacker => attackers.push(i),
            }
        }

        assert!(victim.len() == 1 && honest.len() == 1 && attackers.len() == 18);

        Ok((victim.remove(0), honest.remove(0), attackers))
    }

    async fn play_victim(
        &self,
        discv5: Discv5,
        client: Client,
        honest: &InstanceInfo,
        attackers: &Vec<InstanceInfo>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Wait until the attacker has done its attack.
        client
            .barrier(
                STATE_ATTACKERS_SENT_QUERY,
                u64::try_from(attackers.len()).unwrap(),
            )
            .await?;

        // For debugging, dump the routing table statistics.
        for (i, bucket) in discv5.kbuckets().buckets_iter().enumerate() {
            client.record_message(format!(
                "[KBucket] index:{}, num_entries:{}, num_connected:{}, num_disconnected:{}",
                i,
                bucket.num_entries(),
                bucket.num_connected(),
                bucket.num_disconnected()
            ));
        }

        // If the victim is vulnerable to the eclipse attack, this will result in `Table full`
        // error because the bucket is full of the attacker's node id.
        let result = discv5.add_enr(honest.enr.clone());

        client
            .signal_and_wait(STATE_DONE, client.run_parameters().test_instance_count)
            .await?;

        if let Err(msg) = result {
            client
                .record_failure(format!("Failed to add the honest node's ENR: {}", msg))
                .await?;
        } else {
            client.record_success().await?;
        }
        Ok(())
    }

    async fn play_honest(&self, client: Client) -> Result<(), Box<dyn std::error::Error>> {
        // Nothing to do, just wait until the simulation has been done.
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
        // The victim's ENR is added to the attacker's routing table prior to sending a query. So
        // the FINDNODE query will be sent to the victim, and then, if the victim is vulnerable
        // to the eclipse attack, the attacker's ENR will be added to the victim's routing table
        // because of the handshake.
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

// This function is copied from https://github.com/sigp/discv5/blob/master/src/discv5/test.rs
// Generate `n` deterministic keypairs from a given seed.
fn generate_deterministic_keypair(n: usize, seed: u64) -> Vec<CombinedKey> {
    let mut keypairs = Vec::new();
    for i in 0..n {
        let sk = {
            let rng = &mut rand_xorshift::XorShiftRng::seed_from_u64(seed + i as u64);
            let mut b = [0; 32];
            loop {
                // until a value is given within the curve order
                rng.fill_bytes(&mut b);
                if let Ok(k) = enr::k256::ecdsa::SigningKey::from_bytes(&b) {
                    break k;
                }
            }
        };
        let kp = CombinedKey::from(sk);
        keypairs.push(kp);
    }
    keypairs
}
