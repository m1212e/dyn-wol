use libp2p::{
    gossipsub::{self, TopicHash},
    PeerId,
};
use log::error;
use serde::Deserialize;

pub mod host_info;
pub mod host_occupation;

pub struct ExtractedTopicMessage<T: for<'a> Deserialize<'a>> {
    peer_id: PeerId,
    message: T,
}

pub fn extract_topic_message<T: for<'a> Deserialize<'a>>(
    event: &gossipsub::Event,
    valid_topic_hash: &TopicHash,
) -> Option<ExtractedTopicMessage<T>> {
    match event {
        gossipsub::Event::Message {
            propagation_source: peer_id,
            message_id: _,
            message,
        } => match message.topic.eq(valid_topic_hash) {
            true => {
                let raw: &[u8] = &message.data;
                let r = match flexbuffers::Reader::get_root(raw) {
                    Ok(v) => v,
                    Err(err) => {
                        error!("Could not extract message: {err}");
                        return None;
                    }
                };
                let extracted = match T::deserialize(r) {
                    Ok(v) => v,
                    Err(err) => {
                        error!("Could not extract message: {err}");
                        return None;
                    }
                };
                Some(ExtractedTopicMessage {
                    message: extracted,
                    peer_id: *peer_id,
                })
            }
            false => None,
        },
        _ => None,
    }
}
