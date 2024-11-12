use log::error;
// TODO we dont really need the mac_address crate with sysinfo
use mac_address::MacAddress;
use rand::seq::SliceRandom;

pub fn send_activation_action(
    already_running_mac_addresses: Vec<MacAddress>,
    available_mac_addresses: Vec<MacAddress>,
) {
    let non_stated_mac_addresses = available_mac_addresses
        .iter()
        .filter(|mac_address| !already_running_mac_addresses.contains(mac_address))
        .collect::<Vec<_>>();
    // A random non started mac address
    let mac_address = match non_stated_mac_addresses.choose(&mut rand::thread_rng()) {
        Some(v) => **v,
        None => {
            error!("Could not find any mac address to send the activation action to");
            return;
        }
    };

    // Create a magic packet (but don't send it yet)
    let magic_packet = wake_on_lan::MagicPacket::new(&mac_address.bytes());

    // Send the magic packet via UDP to the broadcast address 255.255.255.255:9 from 0.0.0.0:0
    match magic_packet.send() {
        Ok(v) => v,
        Err(err) => {
            error!("Could not send magic packet: {err:#?}");
        },
    };
}
