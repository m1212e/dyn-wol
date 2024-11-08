use config::read_config;
use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{gossipsub, mdns, swarm::NetworkBehaviour};
use libp2p::{noise, tcp, yamux};
use log::info;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tokio::{io, select};
use topics::extract_topic_message;
use topics::host_info::HostInfoTopic;
use topics::host_info_occupation::HostOccupationTopic;

mod config;
mod topics;

#[derive(NetworkBehaviour)]
pub struct MyBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    info!("Starting dyn-wol");

    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let config = read_config()?;

    info!("Building swarm...");
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|key| {
            let message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(10))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(message_id_fn)
                .build()
                .map_err(|msg| io::Error::new(io::ErrorKind::Other, msg))?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )?;

            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;
            Ok(MyBehaviour { gossipsub, mdns })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();
    info!("Built swarm!");

    swarm.listen_on(
        format!(
            "/ip4/{host_ip}/udp/{port}/quic-v1",
            host_ip = config.host_ip,
            port = config.port
        )
        .parse()?,
    )?;
    swarm.listen_on(
        format!(
            "/ip4/{host_ip}/tcp/{port}",
            host_ip = config.host_ip,
            port = config.port
        )
        .parse()?,
    )?;

    let host_info_topic_instance = HostInfoTopic::register_topic(&mut swarm, &config)?;
    let mut host_occupation_topic_instance =
        HostOccupationTopic::register_topic(&mut swarm, &host_info_topic_instance)?;

    loop {
        select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        info!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        info!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(event)) => {
                    if let Some(message) = extract_topic_message(&event, &host_info_topic_instance.topic_hash) {
                        host_info_topic_instance.handle_incoming_topic_message(message).await;
                    } else if let Some(message) = extract_topic_message(&event, &host_info_topic_instance.topic_hash) {
                        host_occupation_topic_instance.handle_incoming_topic_message(message).await;
                    }
                },
                SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {address}");
                }
                _ => {}
            }
        }
    }
}
