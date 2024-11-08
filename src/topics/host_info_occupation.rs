use std::{collections::HashMap, error::Error};

use libp2p::{
    gossipsub::{self, TopicHash},
    PeerId, Swarm,
};
use log::warn;

use crate::MyBehaviour;

use super::{host_info::HostInfoTopic, ExtractedTopicMessage};

pub struct HostOccupationTopic<'a> {
    topic_hash: TopicHash,
    map: HashMap<PeerId, Occupation>,
    host_info: &'a HostInfoTopic<'a>,
}

struct Occupation {
    cpu_percentage: u8,
    gpu_percentage: u8,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct HostOccupationMessage {
    cpu_percentage: u8,
    gpu_percentage: u8,
}

impl<'a> HostOccupationTopic<'a> {
    pub fn register_topic(
        swarm: &mut Swarm<MyBehaviour>,
        host_info: &'a HostInfoTopic,
    ) -> Result<Self, Box<dyn Error>> {
        let topic = gossipsub::IdentTopic::new("dyn-wol-host-occupation");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        Ok(HostOccupationTopic {
            map: HashMap::new(),
            topic_hash: topic.hash(),
            host_info,
        })
    }

    pub async  fn handle_incoming_topic_message(
        &mut self,
        data: ExtractedTopicMessage<HostOccupationMessage>,
    ) {
        if !self.host_info.peer_id_is_registered(&data.peer_id).await {
            warn!("Got unknown peer id in occupation message, ignoring!");
            return;
        }

        self.map.insert(
            data.peer_id,
            Occupation {
                cpu_percentage: data.message.cpu_percentage,
                gpu_percentage: data.message.gpu_percentage,
            },
        );
    }
}
