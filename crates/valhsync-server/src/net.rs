//! Small network helpers.

use std::net::{IpAddr, UdpSocket};

/// The address this machine would use to reach the internet. Connecting a UDP
/// socket sends nothing; it only asks the kernel for a route.
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
}
