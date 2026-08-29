use anyhow::Result;
use ipnet::Ipv4Net;
use tun_rs::{AsyncDevice, DeviceBuilder};

pub struct TunDev {
    pub dev: AsyncDevice,
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
