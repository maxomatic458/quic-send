use async_compression::tokio::write::{GzipDecoder, GzipEncoder};
use bincode::{Decode, Encode};

use iter_tools::Itertools;
use quinn::Connection;

use std::{fs, io, path::PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, PartialEq, Clone, Encode, Decode, Hash)]
pub enum FileRecvSendTree {
    File {
        name: String,
        skip: u64,
        size: u64,
    },
    Dir {
        name: String,
        files: Vec<FileRecvSendTree>,
    },
}

impl FileRecvSendTree {
    pub fn size(&self) -> u64 {
        match self {
            FileRecvSendTree::File { size, .. } => *size,
            FileRecvSendTree::Dir { files, .. } => files.iter().map(|f| f.size()).sum(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            FileRecvSendTree::File { name, .. } => name.clone(),
            FileRecvSendTree::Dir { name, .. } => name.clone(),
        }
    }

    pub fn skip(&self) -> u64 {
        match self {
            FileRecvSendTree::File { skip, .. } => *skip,
            FileRecvSendTree::Dir { files, .. } => files.iter().map(|f| f.skip()).sum(),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Encode, Decode)]
pub enum FilesToSkip {
    File {
        name: String,
        skip: u64,
    },
    Dir {
        name: String,
        files: Vec<FilesToSkip>,
    },
}

impl FilesToSkip {
    pub fn skip(&self) -> u64 {
        match self {
            FilesToSkip::File { skip, .. } => *skip,
            FilesToSkip::Dir { files, .. } => files.iter().map(|f| f.skip()).sum(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            FilesToSkip::File { name, .. } => name.clone(),
            FilesToSkip::Dir { name, .. } => name.clone(),
        }
    }
}

/// Get the file tree of the files that are available locally
pub fn get_available_files_tree(root: &PathBuf) -> io::Result<FileRecvSendTree> {
    if root.is_file() {
        let size = fs::metadata(root)?.len();
        Ok(FileRecvSendTree::File {
            name: root.file_name().unwrap().to_string_lossy().to_string(),
            skip: 0,
            size,
        })
    } else {
        let name = root.file_name().unwrap().to_string_lossy().to_string();
        let mut files = Vec::new();

        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            files.push(get_available_files_tree(&path)?);
        }

        Ok(FileRecvSendTree::Dir { name, files })
    }
}

/// Get a tree that represents the files that should be skipped (or partially skipped).
/// - ``None`` means the file/directory should not be skipped
pub fn get_files_to_skip_tree(
    local: &FileRecvSendTree,
    remote: &FileRecvSendTree,
) -> Option<FilesToSkip> {
    match (local, remote) {
        (
            FileRecvSendTree::File {
                name: local_name,
                size: local_size,
                ..
            },
            FileRecvSendTree::File {
                name: remote_name, ..
            },
        ) => {
            if local_name != remote_name {
                return None;
            }

            Some(FilesToSkip::File {
                name: remote_name.clone(),
                skip: *local_size,
            })
        }
        (
            FileRecvSendTree::Dir {
                name: local_name,
                files: local_files,
            },
            FileRecvSendTree::Dir {
                name: remote_name,
                files: remote_files,
            },
        ) => {
            if local_name == remote_name {
                let mut files = Vec::new();

                for (local, remote) in local_files.iter().zip(remote_files.iter()) {
                    if let Some(file) = get_files_to_skip_tree(local, remote) {
                        files.push(file);
                    }
                }

                Some(FilesToSkip::Dir {
                    name: remote_name.clone(),
                    files,
                })
            } else {
                None
            }
        }
        _ => {
            tracing::error!("trees do not have the same structure!");
            None
        }
    }
}

/// Apply the [FilesToSkip] tree to the [FileRecvSendTree] tree and return a "stripped" tree
/// - ``None`` means the tree was fully cancelled out, everything should be skipped
pub fn apply_files_to_skip_tree(
    offered: &FileRecvSendTree,
    receiver_skipped: &FilesToSkip,
) -> Option<FileRecvSendTree> {
    match (offered, receiver_skipped) {
        (
            FileRecvSendTree::File {
                name: local_name,
                size: local_size,
                ..
            },
            FilesToSkip::File {
                name: remote_name,
                skip,
            },
        ) => {
            if local_name == remote_name {
                if local_size != skip {
                    Some(FileRecvSendTree::File {
                        name: local_name.clone(),
                        skip: *skip,
                        size: *local_size,
                    })
                } else {
                    None
                }
            } else {
                // should this happen?
                Some(FileRecvSendTree::File {
                    name: local_name.clone(),
                    skip: 0,
                    size: *local_size,
                })
            }
        }
        (
            FileRecvSendTree::Dir {
                name: local_name,
                files: local_files,
            },
            FilesToSkip::Dir {
                name: remote_name,
                files: remote_files,
            },
        ) => {
            if local_name == remote_name {
                let mut files = Vec::new();

                for pair in local_files.iter().zip_longest(remote_files.iter()) {
                    match pair {
                        iter_tools::EitherOrBoth::Both(local, remote) => {
                            if let Some(file) = apply_files_to_skip_tree(local, remote) {
                                files.push(file);
                            }
                        }
                        iter_tools::EitherOrBoth::Left(local) => {
                            files.push(local.clone());
                        }

                        _ => panic!("this should not happen!"),
                    }
                }

                if files.is_empty() {
                    None
                } else {
                    Some(FileRecvSendTree::Dir {
                        name: local_name.clone(),
                        files,
                    })
                }
            } else {
                // should this happen?
                Some(FileRecvSendTree::Dir {
                    name: local_name.clone(),
                    files: local_files.to_owned(),
                })
            }
        }
        _ => {
            tracing::error!("trees do not have the same structure!");
            None
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    #[tokio::test]
    async fn test_compression() {
        let data = b"hello world";
        let compressed = compress_gzip(data).await.unwrap();
        let decompressed = decompress_gzip(&compressed).await.unwrap();

        assert_eq!(data, &decompressed[..]);
    }

    #[test]
    fn test_file_trees() {
        let to_send = FileRecvSendTree::Dir {
            name: "root".to_string(),
            files: vec![
                FileRecvSendTree::File {
                    name: "file1".to_string(),
                    skip: 0,
                    size: 10,
                },
                FileRecvSendTree::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FileRecvSendTree::File {
                            name: "file2".to_string(),
                            skip: 0,
                            size: 20,
                        },
                        FileRecvSendTree::File {
                            name: "file3".to_string(),
                            skip: 0,
                            size: 30,
                        },
                    ],
                },
            ],
        };

        let already_installed = FileRecvSendTree::Dir {
            name: "root".to_string(),
            files: vec![
                FileRecvSendTree::File {
                    name: "file1".to_string(),
                    skip: 0,
                    size: 10,
                },
                FileRecvSendTree::Dir {
                    name: "dir1".to_string(),
                    files: vec![FileRecvSendTree::File {
                        name: "file2".to_string(),
                        skip: 0,
                        size: 15,
                    }],
                },
            ],
        };

        let to_skip = get_files_to_skip_tree(&already_installed, &to_send).unwrap();
        assert_eq!(
            to_skip,
            FilesToSkip::Dir {
                name: "root".to_string(),
                files: vec![
                    FilesToSkip::File {
                        name: "file1".to_string(),
                        skip: 10
                    },
                    FilesToSkip::Dir {
                        name: "dir1".to_string(),
                        files: vec![FilesToSkip::File {
                            name: "file2".to_string(),
                            skip: 15
                        },],
                    },
                ],
            }
        );

        let new_tree_expected = FileRecvSendTree::Dir {
            name: "root".to_string(),
            files: vec![FileRecvSendTree::Dir {
                name: "dir1".to_string(),
                files: vec![FileRecvSendTree::File {
                    name: "file2".to_string(),
                    skip: 15,
                    size: 20,
                },
                FileRecvSendTree::File {
                    name: "file3".to_string(),
                    skip: 0,
                    size: 30,
                }],
            }],
        };

        let new_tree = apply_files_to_skip_tree(&to_send, &to_skip).unwrap();
        assert_eq!(new_tree, new_tree_expected);
    }
}
