# quic-send
A peer-to-peer file sharing application based on QUIC (using the [quinn](https://crates.io/crates/quinn) crate).
You can send files and folders over a direct connection (no relay server involved) to another peer.

Because no third party (except the STUN request) is involved, if the holepunch fails, a connection will not be established. But since QUIC is UDP based, the holepunch should work for most networks.

It might be required to run the program with escalated priviliages.

## Usage
Sender:
```bash
qs send file1.txt file2.txt folder1/
```

Receiver:
```bash
qs receive
```

On both sides you will be presented with a IP address and port number (this is your external IP and Port obtained via STUN). Share this IP with the other peer, enter it in the program and you will be connected.
