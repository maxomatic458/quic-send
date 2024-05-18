use async_compression::tokio::write::{GzipDecoder, GzipEncoder};
use bincode::{Decode, Encode};
use quinn::SendStream;
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Encode, Decode)]
pub enum FileOrDir {
    File { name: String, size: u64 },
    Dir { name: String, sub: Vec<FileOrDir> },
}

impl FileOrDir {
    pub fn size(&self) -> u64 {
        match self {
            Self::File { size, .. } => *size,
            Self::Dir { sub, .. } => sub.iter().map(Self::size).sum(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::File { name, .. } => name,
            Self::Dir { name, .. } => name,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Self::File { .. })
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, Self::Dir { .. })
    }
}

pub async fn send_packet<P: Encode + std::fmt::Debug>(
    packet: P,
    send: &mut SendStream,
) -> io::Result<()> {
    tracing::debug!("Sending packet: {:?}", packet);

    let data = bincode::encode_to_vec(&packet, bincode::config::standard()).unwrap();

    let compressed = compress_gzip(&data).await?;
    let len = compressed.len() as u64;

    send.write_u64(len).await?;
    send.write_all(&compressed).await?;

    Ok(())
}

pub async fn receive_packet<P: Decode + std::fmt::Debug>(
    recv: &mut quinn::RecvStream,
) -> io::Result<P> {
    let len = recv.read_u64().await?;
    let mut compressed = vec![0; len as usize];
    recv.read_exact(&mut compressed).await.unwrap();

    let data = decompress_gzip(&compressed).await?;
    let packet = bincode::decode_from_slice(&data, bincode::config::standard())
        .unwrap()
        .0;

    tracing::debug!("Received packet: {:?}", packet);

    Ok(packet)
}

async fn compress_gzip(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = GzipEncoder::new(&mut out);
    encoder.write_all(data).await?;
    encoder.shutdown().await?;

    Ok(out)
}

async fn decompress_gzip(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut decoder = GzipDecoder::new(&mut out);
    decoder.write_all(data).await?;
    decoder.shutdown().await?;

    Ok(out)
}

pub fn handle_unexpected_packet<T: std::fmt::Debug>(packet: &T) {
    tracing::error!("Received unexpected packet: {:?}", packet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_compression() {
        let data = b"hello world";
        let compressed = compress_gzip(data).await.unwrap();
        let decompressed = decompress_gzip(&compressed).await.unwrap();

        assert_eq!(data, &decompressed[..]);
    }
}
