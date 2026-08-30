use anyhow::Result;
use ipnet::Ipv4Net;
use std::sync::Arc;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tun_rs::{AsyncDevice, DeviceBuilder};

use crate::consts::MAX_FRAME_LEN;

pub struct TunDev {
    dev: AsyncDevice,
}

impl TunDev {
    pub fn new(prefix: Ipv4Net, mtu: u16) -> Result<Self> {
        let dev = DeviceBuilder::new()
            .ipv4(prefix.addr(), prefix.prefix_len(), None)
            .mtu(mtu)
            .build_async()?;

        Ok(Self { dev })
    }

    pub fn dev(&self) -> &AsyncDevice {
        &self.dev
    }
}

pub struct TunReader {
    cancel_token: CancellationToken,
}

impl TunReader {
    pub fn spawn(tun: Arc<TunDev>, to_swarm: Sender<Vec<u8>>) -> Result<(Self, JoinHandle<()>)> {
        let cancel_token = CancellationToken::new();
        let join = tokio::spawn({
            let cancel = cancel_token.clone();
            async move {
                let mut buf = [0u8; MAX_FRAME_LEN];
                loop {
                    tokio::select! {
                         _ = cancel.cancelled() => {
                            info!(target: "vp2pn::tun::reader", "shutting down");
                            break;
                        }
                        res = tun.dev().recv(&mut buf) => {
                            match res {
                                Err(e) => error!(target: "vp2pn::tun::reader", %e, "error reading from tun"),
                                Ok(bytes_read) => {
                                    if bytes_read != 0 {
                                        // REVISIT: don't use vectors so lightly.
                                        let _ = to_swarm.send(buf[..bytes_read].to_vec()).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok((Self { cancel_token }, join))
    }
}

impl Drop for TunReader {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

pub struct TunWriter {
    cancel_token: CancellationToken,
}

impl TunWriter {
    pub fn spawn(
        tun: Arc<TunDev>,
        mut from_swarm: Receiver<Vec<u8>>,
    ) -> Result<(Self, JoinHandle<()>)> {
        let cancel_token = CancellationToken::new();
        let join = tokio::spawn({
            let cancel = cancel_token.clone();
            async move {
                loop {
                    tokio::select! {
                         _ = cancel.cancelled() => {
                            info!(target: "vp2pn::tun::writer", "shutting down");
                            break;
                        }
                        res = from_swarm.recv() => {
                            let Some(packet) = res else {
                                info!(target: "vp2pn::tun::writer", "channel closed, shutting down");
                                break;
                            };
                            if let Err(e) = tun.dev().send(&packet).await {
                                error!(target: "vp2pn::tun::writer", %e, "error writing to tun");
                            }
                        }
                    }
                }
            }
        });

        Ok((Self { cancel_token }, join))
    }
}

impl Drop for TunWriter {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}
