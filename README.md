# vp2pn

A peer-to-peer VPN over libp2p QUIC.

Each side creates a TUN device and forwards every IP packet it reads to the
other peer over a libp2p `request-response` protocol on a QUIC transport. Peers
connect directly, or through a circuit relay v2 server when neither side is
reachable.

## Requirements

- Rust (2024 edition)
- Root privileges — creating a TUN device requires them on both macOS and Linux
- `iperf3` for the throughput test
- Two hosts that can reach each other over UDP, or a relay both can reach

## Build

```bash
cargo build --release
```

The binary lands at `target/release/vp2pn`.

## Usage

```
Usage: vp2pn [OPTIONS] --tun-prefix <TUN_PREFIX> <COMMAND>

Commands:
  server  Listen for incoming peers
  client  Dial a server

Options:
  -t, --tun-prefix <TUN_PREFIX>  tun device address in CIDR form
      --mtu <MTU>                tun device MTU [default: 1400]
```

`--tun-prefix` and `--mtu` belong to the top-level command, so they must appear
**before** `server` or `client`.

| Flag | Scope | Notes |
|---|---|---|
| `--tun-prefix` | global, required | Address *and* prefix, e.g. `20.0.0.1/24`. Both peers must share the subnet and differ in the host part. |
| `--mtu` | global | Must stay below `MAX_FRAME_LEN` (2048). See [MTU](#mtu). |
| `--listen-multiaddr` | `server` | Direct address or relay circuit. Default `/ip4/0.0.0.0/udp/9090/quic-v1`. |
| `--target` | `client`, required | Full server multiaddr, including the `/p2p/<peer-id>` suffix. |

## Testing — direct

### 1. Start the server

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.1/24 --mtu 1400 server
```

It prints one dialable address per network interface:

```
INFO vp2pn::swarm: listening on dial_addr=/ip4/10.0.100.1/udp/9090/quic-v1/p2p/12D3KooWM4ugmTde...
```

Copy the line whose IP the client can actually reach. This is the only place
the server's peer id appears, and the client needs it — a multiaddr without the
`/p2p/...` suffix will not connect.

The peer id is regenerated on every start, so re-copy it each run.

### 2. Start the client

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.2/24 --mtu 1400 client \
  --target "/ip4/10.0.100.1/udp/9090/quic-v1/p2p/12D3KooWM4ugmTde..."
```

Both sides should log `connection established`.

### 3. Check connectivity

```bash
ping 20.0.0.1
```

A reply means packets flow in **both** directions — the request out and the
ICMP echo reply back. One-way breakage shows as 100% packet loss even though
the logs show traffic.

### 4. Throughput

On the server host:

```bash
iperf3 -s
```

On the client host:

```bash
iperf3 -c 20.0.0.1 -t 100
```

`iperf3` connects to the **tunnel** address (`20.0.0.1`), not the underlay
address the multiaddr used. If it connects to the underlay you are measuring
the raw network, not the tunnel.

## Testing — through a relay

Useful when neither peer is directly reachable. Both sides connect out to a
circuit relay v2 server.

### 1. Server reserves a circuit

Pass the **relay's** address with a `/p2p-circuit` suffix. No extra flag — the
address itself selects the mode:

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.1/24 server \
  -l "/ip4/10.0.100.10/udp/50000/quic-v1/p2p/12D3KooWK99VoVxN.../p2p-circuit"
```

Confirm the reservation was granted, and check the limits the relay hands back:

```
INFO vp2pn::swarm: relay reservation accepted relay_peer_id=12D3KooWK99VoVxN... renewal=false
     limit=Some(Limit { duration: Some(4294967295s), data_in_bytes: Some(18446744073709551615) })
```

`renewal=false` is the first reservation; a `renewal=true` line should follow
roughly hourly. If those stop, the circuit address is about to go dead.

The relay echoes back every address *it* is listening on, so you will get one
`listening on` line per relay interface — including its loopback and any
docker/tun addresses. Only the one the client can reach is useful.

### 2. Client dials through the circuit

Append `/p2p/<SERVER_PEER_ID>` to the relay address:

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.2/24 client \
  --target "/ip4/10.0.100.10/udp/50000/quic-v1/p2p/12D3KooWK99VoVxN.../p2p-circuit/p2p/12D3KooWEA6Qi7Ue..."
```

Note the two peer ids: the **relay's** before `/p2p-circuit`, the **server's**
after it. The client makes two connections — one to the relay, one relayed to
the server — so it logs `connection established` twice.

Then test with `ping` and `iperf3` exactly as above.

Relay throughput is not comparable to a direct run: every byte crosses the
relay and is re-encrypted at each hop.

## Logging

Logging defaults to `INFO`. Raise it with `RUST_LOG`:

```bash
sudo RUST_LOG=vp2pn=debug ./target/release/vp2pn ...
```

Two targets, so you can enable them independently:

| Target | Contents |
|---|---|
| `vp2pn::tun` | Packets read from and written to the TUN device |
| `vp2pn::swarm` | Listen addresses, connections, relay reservations, protocol events |

```bash
RUST_LOG=vp2pn=info,vp2pn::tun=debug    # packet flow, quiet swarm
RUST_LOG=vp2pn=info,vp2pn::swarm=debug  # protocol chatter, quiet tun
```

`sudo` scrubs the environment, so `RUST_LOG` must go **inside** the `sudo`
invocation, not exported beforehand.

Per-packet logging is `debug!`. Leave it off for an `iperf3` run — logging
every packet dominates the measurement.

## MTU

The default of 1400 targets a standard 1500-byte Ethernet path. Per-packet
overhead is roughly:

| Layer | Bytes |
|---|---|
| IPv4 header | 20 |
| UDP header | 8 |
| QUIC short header | ~13 |
| QUIC AEAD tag | 16 |
| QUIC STREAM frame header | ~5 |
| length prefix | 4 |
| **total** | **~66** |

Because packets travel over a QUIC *stream* rather than datagrams, exceeding
the path MTU does not drop or fragment anything — QUIC splits and reassembles.
An oversized value costs throughput, not correctness: each tunneled packet then
needs two QUIC packets, and losing either one stalls it.

Use 1280 when the path is not plain Ethernet. It is the IPv6 minimum every path
must support, so it never needs debugging. PPPoE links in particular cap at
1492.

Since cost is paid per *packet*, raising the MTU is a direct lever on bulk
throughput — but `MAX_FRAME_LEN` must be raised to match.

## Tuning

libp2p's defaults are sized for signalling protocols, not per-packet traffic.
[`src/consts.rs`](src/consts.rs) overrides them:

| Constant | Library default | Here | Effect if too low |
|---|---|---|---|
| `MAX_CONCURRENT_STREAMS` | 100 | 1024 | `Dropping inbound stream because we are at capacity` — silent packet loss |
| `CONNECTION_EVENT_BUFFER` | 7 up / 32 down | 1024 | Stalls between connection task and swarm |
| yamux `max_num_streams` | 512 | 2048 | `maximum number of streams reached`, relay path only |
| `MAX_CHANNEL_BOUND` | — | 2048 | Queue depth to the TUN tasks |

Two constraints on raising these further. Yamux asserts
`max_connection_receive_window >= max_num_streams * 256 KiB` against a 1 GiB
default window, so 4096 streams is a hard ceiling — exceeding it panics at
startup. And deep queues are latency, not capacity: a full `MAX_CHANNEL_BOUND`
means packets sit long enough that the inner TCP has already retransmitted them.

The yamux limit applies only to relayed connections. Direct connections
multiplex with QUIC's own streams and never touch yamux.

## Troubleshooting

**`OutboundFailure { error: DialFailure }`** — the peer id being addressed has
no connection. Usually a `--target` missing its `/p2p/...` suffix, or a stale
peer id from a previous server run.

**Circuit closes immediately** — `ConnectionLimits` counts a relayed connection
the same as a direct one, and reaching a peer over a relay costs two outbound
connections rather than one. The limit is derived from whether `--target`
contains `/p2p-circuit`; a direct dial still allows exactly one.

**`Dropping inbound stream because we are at capacity`** — raise
`MAX_CONCURRENT_STREAMS`. Each warning is a dropped packet, so the TCP inside
the tunnel sees loss and backs off.

**`maximum number of streams reached`** (yamux) — relay path only. Raise the
`set_max_num_streams` value in [`src/main.rs`](src/main.rs), within the 4096
ceiling above.

**No TUN device** — on macOS the interface is named `utunN`, not after the
prefix. Confirm with `ifconfig | grep utun`. Both ends need addresses in the
same subnet but different hosts; two peers sharing `20.0.0.1/24` appear to
connect and then route nothing.

**`InvalidData: frame of N bytes exceeds limit`** — `--mtu` was set above
`MAX_FRAME_LEN`. The error surfaces on the *receiving* peer, so check the MTU
on the other side.

**Second client rejected** — intentional. The server sets
`max_established_incoming(1)`, so one tunnel at a time.

## Limitations

- One peer at a time; the server accepts a single inbound connection.
- Identity is ephemeral — a new keypair per process start, so the peer id
  changes on every restart.
- No routing beyond the two tunnel endpoints; the `/24` is not advertised.
- Each packet opens its own QUIC substream and is acknowledged with an empty
  response. The ack is required by `request-response`, not by the tunnel.
- No hole punching. Relayed traffic stays relayed; adding `dcutr` would let it
  upgrade to a direct connection.

## License

MIT — see [LICENSE](LICENSE).
