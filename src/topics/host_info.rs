use std::{collections::HashMap, error::Error, time::Duration};

use crate::{config::AppConfig, MyBehaviour};
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
use wgpu::{Backends, Instance, InstanceDescriptor};

use super::ExtractedTopicMessage;

pub struct HostInfo<'a> {
    config: &'a AppConfig,
    pub topic_hash: TopicHash,
    map: RwLock<HashMap<PeerId, OtherHost>>,
}

struct OtherHost {
    name: String,
    mac_address: MacAddress,
    has_gpu: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct HostInfoMessage {
    token: String,
    mac_address: MacAddress,
    name: String,
    has_gpu: bool,
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
                has_gpu: data.message.has_gpu,
            },
        );
    }

    pub async fn peer_id_is_registered(&self, id: &PeerId) -> bool {
        self.map.read().await.contains_key(id)
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

        let mut instance_desc = InstanceDescriptor::default();
        instance_desc.backends = Backends::all();
        let instance = Instance::new(instance_desc);
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());

        let message = HostInfoMessage {
            mac_address,
            token,
            name,
            has_gpu: adapters.len() > 0,
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
