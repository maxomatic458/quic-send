use std::path::Path;

use async_compression::tokio::write::{GzipDecoder, GzipEncoder};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

/// Tree structure that represents the files that
/// are being sent/received
#[derive(Debug, PartialEq, Clone, Encode, Decode, Hash)]
pub enum FileSendRecvTree {
    File {
        name: String,
        skip: u64,
        size: u64,
    },
    Dir {
        name: String,
        files: Vec<FileSendRecvTree>,
    },
}

impl FileSendRecvTree {
    /// Name of the file or directory
    pub fn name(&self) -> &str {
        match self {
            FileSendRecvTree::File { name, .. } => name,
            FileSendRecvTree::Dir { name, .. } => name,
        }
    }

    /// Size of the tree in bytes
    pub fn size(&self) -> u64 {
        match self {
            FileSendRecvTree::File { size, .. } => *size,
            FileSendRecvTree::Dir { files, .. } => files.iter().map(|f| f.size()).sum(),
        }
    }

    /// Number of bytes being partially skipped
    /// [FileRecvSendTree] does not contain fully skipped files
    pub fn skip(&self) -> u64 {
        match self {
            FileSendRecvTree::File { skip, .. } => *skip,
            FileSendRecvTree::Dir { files, .. } => files.iter().map(|f| f.skip()).sum(),
        }
    }
}

/// Tree structure that represents the files that are available
#[derive(Debug, PartialEq, Clone, Encode, Decode, Hash, Serialize, Deserialize)]
pub enum FilesAvailable {
    File {
        name: String,
        size: u64,
    },
    Dir {
        name: String,
        files: Vec<FilesAvailable>,
    },
}
/// Get the available files
pub fn get_files_available(path: &Path) -> std::io::Result<FilesAvailable> {
    if path.is_file() {
        Ok(FilesAvailable::File {
            name: path.file_name().unwrap().to_str().unwrap().to_string(),
            size: path.metadata()?.len(),
        })
    } else {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            files.push(get_files_available(&path)?);
        }

        Ok(FilesAvailable::Dir {
            name: path.file_name().unwrap().to_str().unwrap().to_string(),
            files,
        })
    }
}

impl FilesAvailable {
    /// Name of the file or directory
    pub fn name(&self) -> &str {
        match self {
            FilesAvailable::File { name, .. } => name,
            FilesAvailable::Dir { name, .. } => name,
        }
    }

    /// Size of the tree in bytes
    pub fn size(&self) -> u64 {
        match self {
            FilesAvailable::File { size, .. } => *size,
            FilesAvailable::Dir { files, .. } => files.iter().map(|f| f.size()).sum(),
        }
    }

    /// Convert the tree to a [FileSendRecvTree]
    pub fn to_send_recv_tree(&self) -> FileSendRecvTree {
        match self {
            FilesAvailable::File { name, size } => FileSendRecvTree::File {
                name: name.to_string(),
                skip: 0,
                size: *size,
            },
            FilesAvailable::Dir { name, files } => FileSendRecvTree::Dir {
                name: name.to_string(),
                files: files.iter().map(|f| f.to_send_recv_tree()).collect(),
            },
        }
    }

    /// Fully/partially remove skipped files from the tree
    /// - Returns [std::option::Option::None] if the tree is fully skipped
    /// - panics if the tree roots do not match
    pub fn remove_skipped(&self, to_skip: &FilesToSkip) -> Option<FileSendRecvTree> {
        match (self, to_skip) {
            (
                FilesAvailable::File { name, size },
                FilesToSkip::File {
                    name: skip_name,
                    skip,
                },
            ) => {
                if name == skip_name && size <= skip {
                    None
                } else {
                    Some(FileSendRecvTree::File {
                        name: name.clone(),
                        skip: *skip,
                        size: *size,
                    })
                }
            }
            (
                FilesAvailable::Dir { name, files },
                FilesToSkip::Dir {
                    name: skip_name,
                    files: skip_files,
                },
            ) => {
                if name != skip_name {
                    panic!("Tree roots do not match");
                }

                let mut remaining_files = Vec::new();
                for file in files {
                    if let Some(skip_file) = skip_files.iter().find(|sf| match (file, sf) {
                        (
                            FilesAvailable::File { name, .. },
                            FilesToSkip::File {
                                name: skip_name, ..
                            },
                        ) => name == skip_name,
                        (
                            FilesAvailable::Dir { name, .. },
                            FilesToSkip::Dir {
                                name: skip_name, ..
                            },
                        ) => name == skip_name,
                        _ => false,
                    }) {
                        if let Some(remaining) = file.remove_skipped(skip_file) {
                            remaining_files.push(remaining);
                        }
                    } else {
                        remaining_files.push(file.clone().to_send_recv_tree());
                    }
                }

                if remaining_files.is_empty() {
                    None
                } else {
                    Some(FileSendRecvTree::Dir {
                        name: name.clone(),
                        files: remaining_files,
                    })
                }
            }
            _ => panic!("Tree roots do not match"),
        }
    }

    /// Compare two trees and return the files that can be skipped.
    /// (e.g. compare local and remote files, returning those that can be skipped during transfer).
    /// it is expected that ``self`` is larger than ``local_files``
    /// # Returns
    /// - [std::option::Option::None] if no files can be skipped
    pub fn get_skippable(&self, local_files: &FilesAvailable) -> Option<FilesToSkip> {
        match (self, local_files) {
            (
                FilesAvailable::File { name, .. },
                FilesAvailable::File {
                    name: local_name,
                    size: local_size,
                },
            ) => {
                if name == local_name {
                    Some(FilesToSkip::File {
                        name: name.clone(),
                        skip: *local_size,
                    })
                } else {
                    None
                }
            }
            (
                FilesAvailable::Dir { name, files },
                FilesAvailable::Dir {
                    name: local_name,
                    files: local_files,
                },
            ) => {
                if name != local_name {
                    return None;
                }

                let mut skippable_files = Vec::new();
                for file in files {
                    if let Some(remote_file) = local_files.iter().find(|rf| match (file, rf) {
                        (
                            FilesAvailable::File { name, .. },
                            FilesAvailable::File {
                                name: local_name, ..
                            },
                        ) => name == local_name,
                        (
                            FilesAvailable::Dir { name, .. },
                            FilesAvailable::Dir {
                                name: local_name, ..
                            },
                        ) => name == local_name,
                        _ => false,
                    }) {
                        if let Some(skippable) = file.get_skippable(remote_file) {
                            skippable_files.push(skippable);
                        }
                    }
                }

                if skippable_files.is_empty() {
                    None
                } else {
                    Some(FilesToSkip::Dir {
                        name: name.clone(),
                        files: skippable_files,
                    })
                }
            }
            _ => None,
        }
    }
}

/// Tree structure that represents files that have been requested for skipping
#[derive(Debug, PartialEq, Clone, Encode, Decode, Hash)]
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
    /// Name of the file or directory
    pub fn name(&self) -> &str {
        match self {
            FilesToSkip::File { name, .. } => name,
            FilesToSkip::Dir { name, .. } => name,
        }
    }

    /// Number of bytes being skipped
    /// This will include fully skipped files
    pub fn skip(&self) -> u64 {
        match self {
            FilesToSkip::File { skip, .. } => *skip,
            FilesToSkip::Dir { files, .. } => files.iter().map(|f| f.skip()).sum(),
        }
    }
}

pub async fn send_packet<P: Encode + std::fmt::Debug>(
    packet: P,
    conn: &iroh::endpoint::Connection,
) -> std::io::Result<()> {
    tracing::debug!("Sending packet: {:?}", packet);
    let mut send = conn.open_uni().await?;

    let data = bincode::encode_to_vec(&packet, bincode::config::standard()).unwrap();
    let compressed = compress_gzip(&data).await?;
    send.write_all(&compressed).await?;

    send.flush().await?;
    send.finish()?;

    Ok(())
}

#[derive(Debug, Error)]
pub enum PacketRecvError {
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("encode error: {0}")]
    EncodeError(#[from] bincode::error::DecodeError),
    #[error("connection error: {0}")]
    Connection(#[from] iroh::endpoint::ConnectionError),
    #[error("read error {0}")]
    Read(#[from] iroh::endpoint::ReadError),
}

pub async fn receive_packet<P: Decode<()> + std::fmt::Debug>(
    conn: &iroh::endpoint::Connection,
) -> Result<P, PacketRecvError> {
    let mut recv = conn.accept_uni().await?;
    let mut buf = Vec::new();

    loop {
        let mut data = vec![0; 1024];
        if let Some(n) = recv.read(&mut data).await? {
            buf.extend_from_slice(&data[..n]);
            continue;
        }

        break;
    }

    let decompressed = decompress_gzip(&buf).await?;

    let packet = bincode::decode_from_slice(&decompressed, bincode::config::standard())?.0;

    tracing::debug!("Received packet: {:?}", packet);

    Ok(packet)
}

async fn compress_gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = GzipEncoder::new(&mut out);
    encoder.write_all(data).await?;
    encoder.shutdown().await?;

    Ok(out)
}

async fn decompress_gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
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
        let data = b"hellllllllllllllllllllllllo world";
        let compressed = compress_gzip(data).await.unwrap();
        let decompressed = decompress_gzip(&compressed).await.unwrap();

        assert!(compressed.len() < data.len());
        assert_eq!(data, &decompressed[..]);
    }

    #[test]
    fn test_file_trees() {
        let files_offered = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![
                FilesAvailable::File {
                    name: "file1".to_string(),
                    size: 10,
                },
                FilesAvailable::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FilesAvailable::File {
                            name: "file2".to_string(),
                            size: 20,
                        },
                        FilesAvailable::File {
                            name: "file3".to_string(),
                            size: 30,
                        },
                    ],
                },
            ],
        };

        let already_installed = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![
                FilesAvailable::File {
                    name: "file1".to_string(),
                    size: 10,
                },
                FilesAvailable::Dir {
                    name: "dir1".to_string(),
                    files: vec![FilesAvailable::File {
                        name: "file2".to_string(),
                        size: 15,
                    }],
                },
            ],
        };

        let to_skip = files_offered.get_skippable(&already_installed).unwrap();
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
                        }],
                    },
                ],
            }
        );

        let new_tree_expected = FileSendRecvTree::Dir {
            name: "root".to_string(),
            files: vec![FileSendRecvTree::Dir {
                name: "dir1".to_string(),
                files: vec![
                    FileSendRecvTree::File {
                        name: "file2".to_string(),
                        skip: 15,
                        size: 20,
                    },
                    FileSendRecvTree::File {
                        name: "file3".to_string(),
                        skip: 0,
                        size: 30,
                    },
                ],
            }],
        };

        let new_tree = files_offered.remove_skipped(&to_skip).unwrap();
        assert_eq!(new_tree, new_tree_expected);
    }

    #[test]
    fn test_no_files_to_skip() {
        let offered = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![
                FilesAvailable::File {
                    name: "file1".to_string(),
                    size: 10,
                },
                FilesAvailable::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FilesAvailable::File {
                            name: "file2".to_string(),
                            size: 20,
                        },
                        FilesAvailable::File {
                            name: "file3".to_string(),
                            size: 30,
                        },
                    ],
                },
            ],
        };

        let installed = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![],
        };

        let to_skip = offered.get_skippable(&installed);
        assert_eq!(to_skip, None);
    }

    #[test]
    fn larger_directory() {
        let offered = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![
                FilesAvailable::File {
                    name: "file1".to_string(),
                    size: 10,
                },
                FilesAvailable::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FilesAvailable::File {
                            name: "file2".to_string(),
                            size: 20,
                        },
                        FilesAvailable::File {
                            name: "file3".to_string(),
                            size: 30,
                        },
                        FilesAvailable::Dir {
                            name: "dir2".to_string(),
                            files: vec![FilesAvailable::File {
                                name: "file4".to_string(),
                                size: 40,
                            }],
                        },
                    ],
                },
                FilesAvailable::Dir {
                    name: "dir3".to_string(),
                    files: vec![FilesAvailable::File {
                        name: "file5".to_string(),
                        size: 50,
                    }],
                },
            ],
        };

        let installed = FilesAvailable::Dir {
            name: "root".to_string(),
            files: vec![
                FilesAvailable::File {
                    name: "file1".to_string(),
                    size: 10,
                },
                FilesAvailable::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FilesAvailable::File {
                            name: "file2".to_string(),
                            size: 5,
                        },
                        FilesAvailable::Dir {
                            name: "dir2".to_string(),
                            files: vec![],
                        },
                    ],
                },
            ],
        };

        let to_skip = offered.get_skippable(&installed).unwrap();
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
                        files: vec![
                            FilesToSkip::File {
                                name: "file2".to_string(),
                                skip: 5
                            },
                            // FilesToSkip::Dir {
                            //     name: "dir2".to_string(),
                            //     files: vec![],
                            // },
                        ],
                    }
                ]
            }
        );

        let new_tree = offered.remove_skipped(&to_skip).unwrap();
        let new_tree_expected = FileSendRecvTree::Dir {
            name: "root".to_string(),
            files: vec![
                FileSendRecvTree::Dir {
                    name: "dir1".to_string(),
                    files: vec![
                        FileSendRecvTree::File {
                            name: "file2".to_string(),
                            skip: 5,
                            size: 20,
                        },
                        FileSendRecvTree::File {
                            name: "file3".to_string(),
                            skip: 0,
                            size: 30,
                        },
                        FileSendRecvTree::Dir {
                            name: "dir2".to_string(),
                            files: vec![FileSendRecvTree::File {
                                name: "file4".to_string(),
                                skip: 0,
                                size: 40,
                            }],
                        },
                    ],
                },
                FileSendRecvTree::Dir {
                    name: "dir3".to_string(),
                    files: vec![FileSendRecvTree::File {
                        name: "file5".to_string(),
                        skip: 0,
                        size: 50,
                    }],
                },
            ],
        };

        assert_eq!(new_tree, new_tree_expected);
    }
}
