use crate::common::{FilesAvailable, FilesToSkip};
use bincode::{Decode, Encode};

/// All packets send from the sender to the receiver
#[derive(Debug, Clone, Encode, Decode)]
pub enum SenderToReceiver {
    /// Initial connection request
    ConnRequest { version_num: String },
    /// Send the files the sender wants to send
    FileInfo { files: Vec<FilesAvailable> },
}

/// All packets send from the receiver to the sender
#[derive(Debug, Clone, Encode, Decode)]
pub enum ReceiverToSender {
    /// Reject the connection request, because the version number is wrong
    WrongVersion { expected: String },
    /// Accept the connection request
    Ok,
    /// Reject the files the sender wants to send
    RejectFiles,
    /// Accept the files, and send the files that are supposed to be fully or partially skipped
    AcceptFilesSkip { files: Vec<Option<FilesToSkip>> },
}
