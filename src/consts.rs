pub const DEFAULT_LISTEN_MULTIADDR: &str = "/ip4/0.0.0.0/udp/9090/quic-v1";
/// Default TUN MTU. `--mtu` also sets the largest tunneled packet the codec
/// accepts: packets ride over a QUIC stream, so the ceiling is the IP maximum
/// (65535, the `u16` range), not the underlay MTU.
pub const DEFAULT_MTU: u16 = 1400;
pub const MAX_CHANNEL_BOUND: usize = 2048;

/// Concurrent request-response substreams per connection, in each direction.
/// The library default of 100 drops inbound streams — and therefore packets —
/// well before the tunnel saturates.
pub const MAX_CONCURRENT_STREAMS: usize = 1024;

/// Buffer between a connection task and the swarm. The library defaults (7 up,
/// 32 down) are sized for signalling protocols, not per-packet traffic.
pub const CONNECTION_EVENT_BUFFER: usize = 1024;
