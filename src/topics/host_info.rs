use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

use crate::{config::AppConfig, MyBehaviour};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use kanal::AsyncSender;
use libp2p::{
    gossipsub::{self, TopicHash},
    PeerId, Swarm,
};
use log::{error, warn};
use mac_address::MacAddress;
use serde::Serialize;
use sysinfo::System;
use tokio::{sync::RwLock, time};

use super::ExtractedTopicMessage;

type MapType = Arc<RwLock<HashMap<PeerId, OtherHost>>>;

pub struct HostInfo<'a> {
    config: &'a AppConfig,
    pub topic_hash: TopicHash,
    map: MapType,
}

pub struct OtherHost {
    name: String,
    pub mac_address: MacAddress,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct HostInfoMessage {
    token_hash: String,
    mac_address: MacAddress,
    name: String,
}

impl<'a> HostInfo<'a> {
    pub fn register(
        swarm: &mut Swarm<MyBehaviour>,
        config: &'a AppConfig,
        sender: &'a AsyncSender<(TopicHash, Vec<u8>)>,
    ) -> Result<Self, Box<dyn Error>> {
        let topic = gossipsub::IdentTopic::new("dyn-wol-host-info");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        // preiodically broadcast our info
        tokio::spawn({
            let cloned_token = config.token.clone();
            let topic_hash = topic.hash();
            let cloned_sender = sender.clone();

            async move {
                let mut interval = time::interval(Duration::from_secs(3));
                loop {
                    interval.tick().await;
                    Self::broadcast_host_info(
                        cloned_token.clone(),
                        cloned_sender.clone(),
                        topic_hash.clone(),
                    )
                    .await;
                }
            }
        });

        Ok(HostInfo {
            config,
            map: Arc::new(RwLock::new(HashMap::new())),
            topic_hash: topic.hash(),
        })
    }

    pub async fn handle_incoming_topic_message(
        &self,
        data: ExtractedTopicMessage<HostInfoMessage>,
        token: &str,
    ) {
        let parsed_hash = match PasswordHash::new(&data.message.token_hash) {
            Ok(v) => v,
            Err(err) => {
                error!("Could not parse hash: {err:#?}");
                return;
            }
        };

        match Argon2::default().verify_password(token.as_bytes(), &parsed_hash) {
            Ok(v) => v,
            Err(err) => {
                error!("Got invalid token in host info message, ignoring! {err:#?}");
                return;
            }
        }

        self.map.write().await.insert(
            data.peer_id,
            OtherHost {
                mac_address: data.message.mac_address,
                name: data.message.name,
            },
        );
    }

    pub async fn peer_id_is_registered(&self, id: &PeerId) -> bool {
        self.map.read().await.contains_key(id)
    }

    pub fn get_map(&self) -> MapType {
        self.map.clone()
    }

    async fn broadcast_host_info(
        token: String,
        sender: AsyncSender<(TopicHash, Vec<u8>)>,
        topic_hash: TopicHash,
    ) {
        let mac_address = match mac_address::get_mac_address() {
            Ok(v) => match v {
                Some(v) => v,
                None => {
                    error!("Got no mac address");
                    return;
                }
            },
            Err(err) => {
                error!("Error reading mac address: {err:#?}");
                return;
            }
        };

        let name = match System::name() {
            Some(v) => v,
            None => {
                error!("Could not get system name");
                return;
            }
        };

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let token_hash = match argon2.hash_password(token.into_bytes().as_ref(), &salt) {
            Ok(v) => v,
            Err(err) => {
                error!("Serialize error: {err:#?}");
                return;
            }
        }
        .to_string();

        let message = HostInfoMessage {
            mac_address,
            token_hash,
            name,
        };

        let mut s = flexbuffers::FlexbufferSerializer::new();
        if let Err(err) = message.serialize(&mut s) {
            error!("Serialize error: {err:#?}");
            return;
        }

        if let Err(err) = sender.send((topic_hash, s.view().into())).await {
            error!("Failed to send {err:#?}");
            return;
        }
    }
}
