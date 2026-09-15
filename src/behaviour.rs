use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{StreamProtocol, connection_limits, futures};
use libp2p_request_response::{self, Codec};
use libp2p_swarm_derive::NetworkBehaviour;
use std::io;

/// Length-prefixed byte frames. `max_frame_len` is the TUN MTU: a packet read
/// from the device can never exceed it, so anything larger from the peer means
/// the two sides run with different `--mtu` values.
#[derive(Clone)]
pub struct BytesCodec {
    max_frame_len: usize,
}

impl BytesCodec {
    pub fn new(max_frame_len: usize) -> Self {
        Self { max_frame_len }
    }
}

async fn read_frame<T>(io: &mut T, max_frame_len: usize) -> io::Result<Vec<u8>>
where
    T: AsyncRead + Unpin + Send,
{
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;

    if len > max_frame_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds limit of {max_frame_len}"),
        ));
    }

    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;

    Ok(buf)
}

async fn write_frame<T>(io: &mut T, data: &[u8]) -> io::Result<()>
where
    T: AsyncWrite + Unpin + Send,
{
    let len: u32 = data
        .len()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame longer than u32::MAX"))?;

    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(data).await?;
    io.close().await
}

#[async_trait::async_trait]
impl Codec for BytesCodec {
    type Protocol = StreamProtocol;
    type Request = Vec<u8>;
    type Response = ();

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_frame(io, self.max_frame_len).await
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        _io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        // read_frame(io).await
        Ok(())
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        data: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_frame(io, &data).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        _io: &mut T,
        _data: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        // write_frame(io, &data).await
        Ok(())
    }
}

#[derive(NetworkBehaviour)]
pub struct Vp2pnBehaviour {
    pub relay_client: libp2p::relay::client::Behaviour,
    pub request_response: libp2p_request_response::Behaviour<BytesCodec>,
    pub ping: libp2p::ping::Behaviour,
    pub limits: connection_limits::Behaviour,
}
