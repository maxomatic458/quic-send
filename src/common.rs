use async_compression::tokio::write::{GzipDecoder, GzipEncoder};
use bincode::{Decode, Encode};

use quinn::Connection;

use std::io;
use tokio::io::AsyncWriteExt;

pub type Blake3Hash = [u8; 32];

#[derive(Debug, Clone, Encode, Decode)]
pub enum FileOrDir {
    File {
        name: String,
        size: u64,
        hash: Option<Blake3Hash>,
    },
    Dir {
        name: String,
        sub: Vec<FileOrDir>,
    },
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
    conn: &Connection,
) -> io::Result<()> {
    tracing::debug!("Sending packet: {:?}", packet);
    let mut send = conn.open_uni().await?;

    let data = bincode::encode_to_vec(&packet, bincode::config::standard()).unwrap();
    let compressed = compress_gzip(&data).await?;
    send.write_all(&compressed).await?;

    send.flush().await?;
    send.finish()?;

    Ok(())
}

pub async fn receive_packet<P: Decode + std::fmt::Debug>(conn: &Connection) -> io::Result<P> {
    let mut recv = conn.accept_uni().await?;
    let mut buf = Vec::new();

    tracing::debug!("Waiting for packet...");

    loop {
        let mut data = vec![0; 1024];
        if let Some(n) = recv.read(&mut data).await? {
            buf.extend_from_slice(&data[..n]);
            continue;
        }

        break;
    }

    let decompressed = decompress_gzip(&buf).await?;

    let packet = bincode::decode_from_slice(&decompressed, bincode::config::standard())
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
