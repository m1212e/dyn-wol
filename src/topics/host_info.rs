use std::{collections::HashMap, error::Error};

use libp2p::{
    gossipsub::{self, TopicHash},
    PeerId, Swarm,
};
use log::{error, warn};
use mac_address::MacAddress;
use serde::Serialize;
use sysinfo::System;
use tokio::sync::RwLock;

use crate::{config::AppConfig, MyBehaviour};

use super::ExtractedTopicMessage;

pub struct HostInfoTopic<'a> {
    config: &'a AppConfig,
    pub topic_hash: TopicHash,
    map: RwLock<HashMap<PeerId, OtherHost>>,
}

struct OtherHost {
    name: String,
    mac_address: MacAddress,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct HostInfoMessage {
    token: String,
    mac_address: MacAddress,
    name: String,
}

impl<'a> HostInfoTopic<'a> {
    pub fn register_topic(
        swarm: &mut Swarm<MyBehaviour>,
        config: &'a AppConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let topic = gossipsub::IdentTopic::new("dyn-wol-host-info");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        Ok(HostInfoTopic {
            config,
            map: RwLock::new(HashMap::new()),
            topic_hash: topic.hash(),
        })
    }

    pub async fn handle_incoming_topic_message(
        &self,
        data: ExtractedTopicMessage<HostInfoMessage>,
    ) {
        if !self.config.token.eq(&data.message.token) {
            warn!("Got invalid token in host info message, ignoring!");
            return;
        }

        self.map.write().await.insert(
            data.peer_id,
            OtherHost {
                mac_address: data.message.mac_address,
                name: data.message.name,
            },
        );
    }

    pub async fn broadcast_host_info(&self, swarm: &mut Swarm<MyBehaviour>) {
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

        let message = HostInfoMessage {
            mac_address,
            token: self.config.token.clone(),
            name,
        };

        let mut s = flexbuffers::FlexbufferSerializer::new();
        if let Err(err) = message.serialize(&mut s) {
            error!("Serialize error: {err:#?}");
            return;
        }

        if let Err(e) = swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic_hash.clone(), s.view())
        {
            error!("Publish error: {e:#?}");
            return;
        }
    }

    pub async fn peer_id_is_registered(&self, id: &PeerId) -> bool {
        self.map.read().await.contains_key(id)
    }
}
