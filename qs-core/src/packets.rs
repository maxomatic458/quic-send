use std::net::SocketAddr;

use crate::{
    common::{FilesAvailable, FilesToSkip},
    CODE_LEN,
};
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

/// Incoming packet for the roundezvous server
#[derive(Debug, Decode, Encode)]
pub enum RoundezvousToServer {
    /// Sender announces itself to the roundezvous server
    Announce {
        /// The sender's qs version
        version: String,
        /// The external socket addr of the sender
        socket_addr: SocketAddr,
    },
    /// Receiver connects to the roundezvous server
    Connect {
        /// The receiver's qs version
        version: String,
        /// The external socket addr of the receiver
        socket_addr: SocketAddr,
        /// The code the receiver received from the sender
        code: [u8; CODE_LEN],
    },
}

/// Outgoing packet for the roundezvous server
#[derive(Debug, Decode, Encode)]
pub enum RoundezvousFromServer {
    /// Send the code to the sender after it announced itself
    Code { code: [u8; CODE_LEN] },
    /// Exchange the IPs of the sender and receiver
    SocketAddr { socket_addr: SocketAddr },
    /// Reject the connection request, because the version number is wrong
    WrongVersion { expected: String },
}
