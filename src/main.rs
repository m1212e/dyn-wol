use config::read_config;
use futures::StreamExt;
use libp2p::gossipsub::TopicHash;
use libp2p::swarm::SwarmEvent;
use libp2p::{gossipsub, mdns, swarm::NetworkBehaviour};
use libp2p::{noise, tcp, yamux};
use log::{error, info};
use send_activation_action::send_activation_action;
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tokio::{io, select};
use topics::extract_topic_message;
use topics::host_info::HostInfo;
use topics::host_occupation::HostOccupation;

mod config;
mod send_activation_action;
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

    let (incoming_sender, incoming_receiver) = kanal::unbounded_async::<gossipsub::Event>();
    let (outgoing_sender, outgoing_receiver) = kanal::unbounded_async::<(TopicHash, Vec<u8>)>();

    let host_info_instance = HostInfo::register(&mut swarm, &config, &outgoing_sender)?;
    let host_occupation_instance =
        HostOccupation::register(&mut swarm, &outgoing_sender, &host_info_instance)?;

    tokio::spawn({
        let occupation_map = host_occupation_instance.get_map();
        let info_map = host_info_instance.get_map();
        let occupation_level_percentage_threshold = config.occupation_level_percentage;
        let our_peer_id = swarm.local_peer_id().clone();
        let config = config.clone();
        async move {
            loop {
                let total = HostOccupation::calculate_total_occupation(&occupation_map)
                    .await
                    .cpu_percentage;

                let occupation_map_lock = occupation_map.read().await;
                let info_map_lock = info_map.read().await;
                if total > occupation_level_percentage_threshold as f32 {
                    info!("Occupation level is too high: {total}");
                    let lowest_other_peer_id =
                        occupation_map_lock.iter().map(|v| v.0.to_string()).min();

                    match lowest_other_peer_id {
                        Some(lowest) => {
                            if lowest > our_peer_id.to_string() {
                                continue;
                            }
                        }
                        None => {}
                    }

                    send_activation_action(
                        info_map_lock
                            .iter()
                            .map(|v| v.1.mac_address)
                            .collect::<Vec<_>>(),
                        config
                            .hosts
                            .iter()
                            .map(|v| v.mac_address)
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
    });

    tokio::spawn(async move {
        loop {
            select! {
                outgoing = outgoing_receiver.recv() => match outgoing {
                    Ok(v) => {
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(v.0, v.1) {
                            error!("Swarm publish error: {e:?}");
                        }
                    },
                    Err(err) => error!("Could not listen for outgoing: {err:#?}"),
                },
                event = swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _multiaddr) in list {
                            info!("mDNS discovered a new peer: {peer_id}");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        }
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _multiaddr) in list {
                            info!("mDNS discover peer has expired: {peer_id}");
                            swarm
                                .behaviour_mut()
                                .gossipsub
                                .remove_explicit_peer(&peer_id);
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {address}");
                    }
                    SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(event)) => {
                        match incoming_sender.send(event).await {
                            Ok(v) => v,
                            Err(err) => {
                                error!("Could not send on incoming: {err:#?}");
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    loop {
        match incoming_receiver.recv().await {
            Ok(incoming) => {
                if let Some(message) =
                    extract_topic_message(&incoming, &host_info_instance.topic_hash)
                {
                    host_info_instance
                        .handle_incoming_topic_message(message, &config.token)
                        .await;
                } else if let Some(message) =
                    extract_topic_message(&incoming, &host_occupation_instance.topic_hash)
                {
                    host_occupation_instance
                        .handle_incoming_topic_message(message)
                        .await;
                }
            }
            Err(err) => error!("Could not receive incoming message: {err:#?}"),
        }
    }
}
