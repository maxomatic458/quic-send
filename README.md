# quic-send

quic-send is a peer-to-peer file transfer tool that uses the QUIC protocol to transfer data over a direct connection (no relay involved).

> [!NOTE]
> Because quic-send uses NAT-traversal, a direct connection might not be possible on every network. 

![demo](https://github.com/user-attachments/assets/4e3e648e-a3c5-495e-ae0e-4447b2ccfed8)

## Features
- **P2P Data transfer**: All files are sent over a direct connection, the data is never relayed through another server. The only other parties
involved is a STUN server (Google STUN) and an optional roundezvous server (included in this repo).
- **Encryption**: quic-send uses the encryption provided by the [quinn](https://crates.io/crates/quinn) crate (which uses [rustls](https://crates.io/crates/rustls) and [ring](https://crates.io/crates/ring) under the hood).
- **Resumable transfers**: If the connection is lost, the transfer can be resumed from where it left off.
- **Transfer files & Folders**
- **No port forwarding required**: quic-send makes use of [UDP hole punching](https://en.wikipedia.org/wiki/UDP_hole_punching) to establish a connection between the two peers, without requiring open ports.

## Installation

The cli version can be downloaded from crates.io:

```bash
cargo install qs-cli
```

Downloads to the gui version (as well as the cli version) can be found on the [releases page](https://github.com/maxomatic458/quic-send/releases)

## Usage

### Sending files

```
$ qs send <file/folder>
code: 123456
on the other peer, run the following command:

qs receive 123456
```

### Receiving files

```
$ qs receive 123456
```


## Comparison with other file transfer tools
| Feature | quic-send | [Magic Wormhole](https://github.com/magic-wormhole/magic-wormhole) | [croc](https://github.com/schollz/croc) |
|---------|-----------|--------------------------------------------------------------------|-----------------------------------------|
| Encryption | ✅ | ✅ | ✅ |
| Direct (P2P) transfer  | ✅ | (✅)* | ❌ |
| Resumable transfers | ✅ | ❌ | ✅ |
| Transfer files & Folders | ✅ | ✅ | ✅ |
| (fallback) Relay server | ❌ | ✅ | ✅ |

* While it is possible in Magic Wormhole, establishing a direct connection is very unlikely (as the connection tries to establish a direct TCP connection), quick send uses UDP hole punching instead which is way more reliable and works for most networks.

The icon of the gui is made with the font [Commodore 64 Rounded](https://online-fonts.com/fonts/commodore-64-rounded) by Devin Cook.