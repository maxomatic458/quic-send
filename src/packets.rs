use crate::common::{Blake3Hash, FileOrDir};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Encode, Decode)]
pub enum ClientPacket {
    ConnRequest {
        version_num: String,
    },
    FileMeta {
        files: Vec<FileOrDir>,
    },
    /// Response to a [`ServerPacket::FileFromPos`] request
    FilePosHash {
        hash: Option<Blake3Hash>,
    },
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum ServerPacket {
    /// generic OK response
    Ok,
    /// The server is running a different version
    WrongVersion { expected: String },
    /// [`ClientPacket::FileMeta`] was accepted
    AcceptFiles,
    /// [`ClientPacket::FileMeta`] was rejected
    RejectFiles,
    /// [`crate::server::SaveMode::SkipIfNotExists`]
    SkipFile,
    /// If [`crate::server::SaveMode::Append`] is used,
    /// the server will request the file to be sent from a specific
    /// position. The server will expect a [`ClientPacket::FilePosHash`]
    FileFromPos { pos: u64 },
}
