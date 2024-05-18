use crate::common::FileOrDir;
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Encode, Decode)]
pub enum ClientPacket {
    ConnRequest { version_num: String },
    FileMeta { files: Vec<FileOrDir> },
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum ServerPacket {
    Ok,
    WrongVersion { expected: String },
    AcceptFiles,
    RejectFiles,
}
