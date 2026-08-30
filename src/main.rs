use anyhow::Result;
use libp2p::connection_limits::{self, ConnectionLimits};
use libp2p::{futures::StreamExt, identity::Keypair, multiaddr::Protocol, swarm::SwarmEvent};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use vp2pn::behaviour::{BytesCodec, Vp2pnBehaviour, Vp2pnBehaviourEvent};
use vp2pn::config::{Config, Mode};
use vp2pn::consts::MAX_CHANNEL_BOUND;
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

    let limits = match config.params.mode {
        Mode::Server { .. } => ConnectionLimits::default()
            .with_max_established_incoming(Some(1))
            .with_max_pending_incoming(Some(1)),
        Mode::Client { .. } => ConnectionLimits::default().with_max_established_outgoing(Some(1)),
    };

    // Starting with req_res for the test but better be no responses.
    let request_response = libp2p_request_response::Behaviour::with_codec(
        BytesCodec,
        [(
            libp2p::StreamProtocol::new("/reqres_bytes/1"),
            libp2p_request_response::ProtocolSupport::Full,
        )],
        libp2p_request_response::Config::default(),
    );

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|_| Vp2pnBehaviour {
            request_response,
            ping: libp2p::ping::Behaviour::default(),
            limits: connection_limits::Behaviour::new(limits),
        })?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)))
        .build();

    match config.params.mode {
        vp2pn::config::Mode::Server { port } => {
            let listen_multiaddr = format!("/ip4/0.0.0.0/udp/{port}/quic-v1").parse()?;
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
                        let dial_addr = address.with(Protocol::P2p(*swarm.local_peer_id()));
                        info!(target: "vp2pn::swarm", %dial_addr, "listening on");
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
                    _ => {}
                }
            }
        }
    }

    drop(tun_reader);
    join_tun_reader.await?;

    drop(tun_writer);
    join_tun_writer.await?;

    Ok(())
}
