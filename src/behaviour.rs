use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{StreamProtocol, connection_limits, futures};
use libp2p_request_response::{self, Codec};
use libp2p_swarm_derive::NetworkBehaviour;
use std::io;

use crate::consts::MAX_FRAME_LEN;

#[derive(Clone, Default)]
pub struct BytesCodec;

async fn read_frame<T>(io: &mut T) -> io::Result<Vec<u8>>
where
    T: AsyncRead + Unpin + Send,
{
    let mut len = [0u8; 4];
    io.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;

    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds limit of {MAX_FRAME_LEN}"),
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
        read_frame(io).await
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
    pub request_response: libp2p_request_response::Behaviour<BytesCodec>,
    pub ping: libp2p::ping::Behaviour,
    pub limits: connection_limits::Behaviour,
}
