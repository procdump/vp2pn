use clap::{Parser, Subcommand};
use ipnet::Ipv4Net;
use libp2p::Multiaddr;

use crate::consts::{DEFAULT_LISTEN_MULTIADDR, DEFAULT_MTU};

#[derive(Parser, Debug)]
pub struct CommandLineParams {
    /// tun device address in CIDR form
    #[arg(short, long)]
    pub tun_prefix: Ipv4Net,

    /// tun device MTU
    #[arg(long, default_value_t = DEFAULT_MTU)]
    pub mtu: u16,

    #[command(subcommand)]
    pub mode: Mode,
}

#[derive(Subcommand, Debug)]
pub enum Mode {
    /// Listen for incoming peers
    Server {
        /// Where to listen. Either a direct address
        /// (/ip4/0.0.0.0/udp/9090/quic-v1) or a relay circuit
        /// (/ip4/<relay>/udp/4001/quic-v1/p2p/<RELAY_ID>/p2p-circuit).
        #[arg(short, long, default_value = DEFAULT_LISTEN_MULTIADDR)]
        listen_multiaddr: Multiaddr,
    },
    /// Dial a server
    Client {
        /// Server multiaddr, e.g. /ip4/127.0.0.1/udp/9090/quic-v1/p2p/12D3Koo...
        #[arg(short, long)]
        target: Multiaddr,
    },
}

pub struct Config {
    pub params: CommandLineParams,
}

impl Config {
    pub fn new() -> Self {
        Self {
            params: CommandLineParams::parse(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}
