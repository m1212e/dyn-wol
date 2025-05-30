use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

use crate::MyBehaviour;
use kanal::AsyncSender;
use libp2p::{
    gossipsub::{self, TopicHash},
    PeerId, Swarm,
};
use log::{error, warn};
use serde::Serialize;
use sysinfo::System;
use tokio::{sync::RwLock, time};

use super::{host_info::HostInfo, ExtractedTopicMessage};

type MapType = Arc<RwLock<HashMap<PeerId, OtherHostOccupation>>>;

pub struct HostOccupation<'a> {
    pub topic_hash: TopicHash,
    map: MapType,
    host_info: &'a HostInfo<'a>,
}

pub struct OtherHostOccupation {
    cpu_percentage: f32,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct HostOccupationMessage {
    pub cpu_percentage: f32,
}

impl<'a> HostOccupation<'a> {
    pub fn register(
        swarm: &mut Swarm<MyBehaviour>,
        sender: &'a AsyncSender<(TopicHash, Vec<u8>)>,
        host_info: &'a HostInfo,
    ) -> Result<Self, Box<dyn Error>> {
        let topic = gossipsub::IdentTopic::new("dyn-wol-host-occupation");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        // preiodically broadcast our occupation
        tokio::spawn({
            let topic_hash = topic.hash();
            let cloned_sender = sender.clone();

            async move {
                let mut interval = time::interval(Duration::from_secs(3));
                loop {
                    interval.tick().await;
                    Self::broadcast_host_occupation(cloned_sender.clone(), topic_hash.clone())
                        .await;
                }
            }
        });

        Ok(HostOccupation {
            host_info,
            map: Arc::new(RwLock::new(HashMap::new())),
            topic_hash: topic.hash(),
        })
    }

    pub async fn handle_incoming_topic_message(
        &self,
        data: ExtractedTopicMessage<HostOccupationMessage>,
    ) {
        if !self.host_info.peer_id_is_registered(&data.peer_id).await {
            warn!("Got occupation message from non registered peer!");
            return;
        }

        self.map.write().await.insert(
            data.peer_id,
            OtherHostOccupation {
                cpu_percentage: data.message.cpu_percentage,
            },
        );
    }

    async fn broadcast_host_occupation(
        sender: AsyncSender<(TopicHash, Vec<u8>)>,
        topic_hash: TopicHash,
    ) {
        let mut sys = System::new_all();
        sys.refresh_all();

        let message = HostOccupationMessage {
            cpu_percentage: sys.global_cpu_usage(),
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

    pub fn get_map(&self) -> MapType {
        self.map.clone()
    }

    pub async fn calculate_total_occupation(map: &MapType) -> HostOccupationMessage {
        let mut sys = System::new_all();
        sys.refresh_all();

        let others = map.read().await;
        let mut total_occupation = sys.global_cpu_usage();

        for (_, other_host_occupation) in others.iter() {
            total_occupation += other_host_occupation.cpu_percentage;
        }

        HostOccupationMessage {
            cpu_percentage: total_occupation / (others.len() as f32 + 1f32),
        }
    }
}
