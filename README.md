# vp2pn

A peer-to-peer VPN over libp2p QUIC.

Each side creates a TUN device and forwards every IP packet it reads to the
other peer over a libp2p `request-response` protocol on a QUIC transport.

## Requirements

- Rust (2024 edition)
- Root privileges — creating a TUN device requires them on both macOS and Linux
- `iperf3` for the throughput test
- Two hosts that can reach each other over UDP, or one host for a loopback test

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
| `--port` | `server` | UDP port to listen on. Default 9090. |
| `--target` | `client`, required | Full server multiaddr, including the `/p2p/<peer-id>` suffix. |

## Testing

### 1. Start the server

On the machine that will listen:

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.1/24 --mtu 1400 server
```

It prints one dialable address per network interface:

```
INFO vp2pn::swarm: listening on dial_addr=/ip4/10.0.100.1/udp/9090/quic-v1/p2p/12D3KooWM4ugmTde35EJn8CwspfXSyGNm76htA9Q9berYsrzrF33
```

Copy the line whose IP the client can actually reach. This address is the only
place the server's peer id is shown, and the client needs it — a multiaddr
without the `/p2p/...` suffix will not connect.

The peer id is regenerated on every start, so re-copy it each run.

### 2. Start the client

On the other machine:

```bash
sudo ./target/release/vp2pn --tun-prefix 20.0.0.2/24 --mtu 1400 client \
  --target "/ip4/10.0.100.1/udp/9090/quic-v1/p2p/12D3KooWM4ugmTde35EJn8CwspfXSyGNm76htA9Q9berYsrzrF33"
```

Both sides should log:

```
INFO vp2pn::swarm: connection established peer_id=12D3KooW...
```

### 3. Check connectivity

From the client:

```bash
ping 20.0.0.1
```

A reply means packets are flowing in **both** directions — the request out and
the ICMP echo reply back. One-way breakage shows up as 100% packet loss even
though the logs show traffic.

### 4. Throughput with iperf3

On the server host:

```bash
iperf3 -s
```

On the client host:

```bash
iperf3 -c 20.0.0.1 -t 100
```

Note that `iperf3` connects to the **tunnel** address (`20.0.0.1`), not the
underlay address the multiaddr used (`10.0.100.1`). If it connects to the
underlay you are measuring the raw network, not the tunnel.

## Logging

Logging is off below `INFO` by default. Raise it with `RUST_LOG`:

```bash
sudo RUST_LOG=vp2pn=debug ./target/release/vp2pn ...
```

Two targets are used, so you can enable them independently:

| Target | Contents |
|---|---|
| `vp2pn::tun` | Packets read from and written to the TUN device |
| `vp2pn::swarm` | Listen addresses, connections, protocol events |

```bash
RUST_LOG=vp2pn=info,vp2pn::tun=debug    # packet flow, quiet swarm
RUST_LOG=vp2pn=info,vp2pn::swarm=debug  # protocol chatter, quiet tun
```

`sudo` scrubs the environment, so `RUST_LOG` must go **inside** the `sudo`
invocation as shown above, not exported beforehand.

Per-packet logging is `debug!`, so it is invisible at the default level. Leave
it that way for an `iperf3` run — logging every packet dominates the
measurement.

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

## Troubleshooting

**`OutboundFailure { error: DialFailure }`** — the peer id being addressed has
no connection. On the client this usually means the `--target` multiaddr was
missing its `/p2p/...` suffix or carried a stale peer id from a previous server
run.

**No TUN device** — on macOS the interface is named `utunN`, not after the
prefix. Confirm with `ifconfig | grep utun`. Both ends need addresses in the
same subnet but different hosts; two peers sharing `20.0.0.1/24` will appear to
connect and then route nothing.

**`InvalidData: frame of N bytes exceeds limit`** — `--mtu` was set above
`MAX_FRAME_LEN` (2048). The error surfaces on the *receiving* peer, so check
the MTU on the other side.

**Second client rejected** — intentional. The server sets
`max_established_incoming(1)`, so exactly one tunnel is allowed at a time.
Rejections appear as `SwarmEvent::IncomingConnectionError`.

## Limitations

- One peer at a time; the server accepts a single inbound connection.
- Identity is ephemeral — a new keypair per process start, so the peer id
  changes on every restart.
- No routing beyond the two tunnel endpoints; the `/24` is not advertised
  anywhere.
- Each packet opens its own QUIC substream and is acknowledged with an empty
  response.

## License

MIT — see [LICENSE](LICENSE).
