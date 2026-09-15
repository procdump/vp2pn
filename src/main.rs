use anyhow::Result;
use libp2p::connection_limits::{self, ConnectionLimits};
use libp2p::{futures::StreamExt, identity::Keypair, multiaddr::Protocol, swarm::SwarmEvent};
use libp2p::{noise, relay, yamux};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use vp2pn::behaviour::{BytesCodec, Vp2pnBehaviour, Vp2pnBehaviourEvent};
use vp2pn::config::{Config, Mode};
use vp2pn::consts::{CONNECTION_EVENT_BUFFER, MAX_CHANNEL_BOUND, MAX_CONCURRENT_STREAMS};
use vp2pn::tun::{TunDev, TunReader, TunWriter};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::new();

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init();

    let keypair = Keypair::generate_ed25519();
    let tun = Arc::new(TunDev::new(config.params.tun_prefix, config.params.mtu)?);

    let limits = match &config.params.mode {
        Mode::Server { .. } => ConnectionLimits::default()
            .with_max_established_incoming(Some(1))
            .with_max_pending_incoming(Some(1)),
        Mode::Client { target } => {
            // Reaching the server over a circuit costs an extra connection: one
            // to the relay, plus the relayed one. `ConnectionLimits` counts a
            // relayed connection the same as a direct one.
            let via_relay = target.iter().any(|p| matches!(p, Protocol::P2pCircuit));
            ConnectionLimits::default().with_max_established_outgoing(Some(if via_relay {
                2
            } else {
                1
            }))
        }
    };

    // Starting with req_res for the test but better be no responses.
    let request_response = libp2p_request_response::Behaviour::with_codec(
        BytesCodec::new(usize::from(config.params.mtu)),
        [(
            libp2p::StreamProtocol::new("/reqres_bytes/1"),
            libp2p_request_response::ProtocolSupport::Full,
        )],
        libp2p_request_response::Config::default()
            .with_max_concurrent_streams(MAX_CONCURRENT_STREAMS),
    );

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        // A relayed connection is noise + yamux inside a QUIC stream, so its
        // substreams are yamux streams, not QUIC ones. Yamux caps them at 512
        // by default and counts both directions against the same limit.
        .with_relay_client(noise::Config::new, || {
            let mut cfg = yamux::Config::default();
            cfg.set_max_num_streams(MAX_CONCURRENT_STREAMS * 2);
            cfg
        })?
        .with_behaviour(|_key, relay_client| Vp2pnBehaviour {
            relay_client,
            request_response,
            ping: libp2p::ping::Behaviour::default(),
            limits: connection_limits::Behaviour::new(limits),
        })?
        .with_swarm_config(|cfg| {
            cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX))
                .with_max_negotiating_inbound_streams(MAX_CONCURRENT_STREAMS)
                .with_per_connection_event_buffer_size(CONNECTION_EVENT_BUFFER)
                .with_notify_handler_buffer_size(
                    NonZeroUsize::new(CONNECTION_EVENT_BUFFER).expect("non-zero"),
                )
        })
        .build();

    match config.params.mode {
        vp2pn::config::Mode::Server { listen_multiaddr } => {
            info!(target: "vp2pn::swarm", %listen_multiaddr, "listening on");
            swarm.listen_on(listen_multiaddr)?;
        }
        vp2pn::config::Mode::Client { target } => {
            info!(target: "vp2pn::swarm", %target, "connecting to target");
            swarm.dial(target)?;
        }
    }

    let mut remote_peer_id = None;

    let (tun_reader_tx, mut tun_reader_rx) = tokio::sync::mpsc::channel(MAX_CHANNEL_BOUND);
    let (tun_reader, join_tun_reader) = TunReader::spawn(tun.clone(), tun_reader_tx)?;

    let (tun_writer_tx, tun_writer_rx) = tokio::sync::mpsc::channel(MAX_CHANNEL_BOUND);
    let (tun_writer, join_tun_writer) = TunWriter::spawn(tun, tun_writer_rx)?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }

            res = tun_reader_rx.recv() => {
                let Some(packet) = res else { break };
                if let Some(remote_peer_id) = remote_peer_id {
                    let _outbound_reqid = swarm.behaviour_mut().request_response.send_request(&remote_peer_id, packet);
                }
            }

            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        // A circuit address already ends with `/p2p/<self>`; a
                        // direct one ends at the transport and needs it added.
                        let dial_addr = match address.iter().last() {
                            Some(Protocol::P2p(_)) => address,
                            _ => address.with(Protocol::P2p(*swarm.local_peer_id())),
                        };
                        info!(target: "vp2pn::swarm", %dial_addr, "listening on");
                    },
                    SwarmEvent::Behaviour(Vp2pnBehaviourEvent::RelayClient(
                        relay::client::Event::ReservationReqAccepted { relay_peer_id, renewal, limit },
                    )) => {
                        info!(
                            target: "vp2pn::swarm",
                            %relay_peer_id,
                            %renewal,
                            ?limit,
                            "relay reservation accepted"
                        );
                    },
                    SwarmEvent::Behaviour(Vp2pnBehaviourEvent::RequestResponse(
                        libp2p_request_response::Event::Message { message, .. },
                    )) => match message {
                        libp2p_request_response::Message::Request { request, channel, .. } => {
                            let req_len = request.len();
                            debug!(target: "vp2pn::swarm", %req_len, "forwarding len packet to tun");
                            if let Err(e) = tun_writer_tx.try_send(request) {
                                debug!(target: "vp2pn::swarm", %e, "tun writer full, dropping packet");
                            }
                            let _ = swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(channel, ());
                        }
                        libp2p_request_response::Message::Response { .. } => {}
                    },
                    SwarmEvent::Behaviour(event) => {
                        debug!(target: "vp2pn::swarm", ?event, "got swarm event");
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        info!(target: "vp2pn::swarm", %peer_id, "connection established");
                        remote_peer_id = Some(peer_id);
                    }
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        info!(target: "vp2pn::swarm", %peer_id, "connection closed");
                        remote_peer_id = None;
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        error!(target: "vp2pn::swarm", ?peer_id, %error, "outgoing connection failed");
                    }
                    SwarmEvent::IncomingConnectionError { error, .. } => {
                        error!(target: "vp2pn::swarm", %error, "incoming connection failed");
                    }
                    _ => {}
                }
            }
        }
    }

    drop(tun_reader);
    join_tun_reader.await?;

    drop(tun_writer);
    join_tun_writer.await?;

    info!(target: "vp2pn::swarm", "exiting");

    Ok(())
}
