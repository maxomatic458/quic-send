use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{atomic::AtomicU64, Arc},
    time,
};

use clap::Parser;
use qs_core::{
    common::{receive_packet, send_packet},
    packets::{RoundezvousFromServer, RoundezvousToServer},
    utils::{self_signed_cert, Version},
    CODE_LEN, KEEP_ALIVE_INTERVAL_SECS, VERSION,
};

use quinn::{Connection, ServerConfig};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("invalid code {0:?}")]
    InvalidCode([u8; CODE_LEN]),
    #[error("wrong version, expected {0}, got {1}")]
    WrongVersion(Version, Version),
}

const DEFAULT_BIND_PORT: u16 = 9090;
const CODE_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const MAX_CONNECTION_AGE: u64 = 60;
const MAX_CONCURRENT_CONNECTIONS: usize = 1_000;

#[derive(Parser, Debug)]
#[clap(version = VERSION, author = env!("CARGO_PKG_AUTHORS"))]
struct Args {
    /// Log level
    #[clap(long, short, default_value = "info")]
    log_level: tracing::Level,
    /// Port
    #[clap(long, short, default_value_t = DEFAULT_BIND_PORT)]
    port: u16,
    /// bind ip
    #[clap(long, short = 's', default_value_t = IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    bind_ip: IpAddr,
    /// Max connection age
    #[clap(long, short, default_value_t = MAX_CONNECTION_AGE)]
    max_connection_age: u64,
    /// Max concurrent connections
    #[clap(long, short = 'c', default_value_t = MAX_CONCURRENT_CONNECTIONS)]
    max_concurrent_connections: usize,
}

struct AppState {
    /// Keep track of the connections awaiting exchange
    awaiting_exchange: RwLock<HashMap<[u8; CODE_LEN], (Connection, SocketAddr, u64)>>,
    /// Counter for the connections
    counter: Arc<AtomicU64>,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let args: Args = Args::parse();

    tracing_subscriber::fmt()
        .with_max_level(args.log_level)
        .init();

    let addr = SocketAddr::new(args.bind_ip, args.port);
    let endpoint = quinn::Endpoint::server(server_config(), addr)?;

    let state = Arc::new(AppState {
        awaiting_exchange: RwLock::new(HashMap::new()),
        counter: Arc::new(AtomicU64::new(0)),
    });

    tracing::info!("started roundezvous server on {}", addr);

    loop {
        if let Some(conn) = endpoint.accept().await {
            if state.awaiting_exchange.read().await.len() >= args.max_concurrent_connections {
                tracing::warn!("max concurrent connections reached, dropping connection");
                continue;
            }

            match conn.await {
                Ok(conn) => {
                    tokio::spawn(handle_connection(conn, state.clone()));
                }
                Err(e) => {
                    tracing::warn!("error accepting connection: {:?}", e);
                }
            }
        }
    }
}

async fn handle_connection(conn: Connection, state: Arc<AppState>) -> Result<(), AppError> {
    match receive_packet::<RoundezvousToServer>(&conn).await? {
        RoundezvousToServer::Announce {
            version,
            socket_addr,
        } => {
            validate_version(version, &conn).await?;

            let code = generate_code();
            let conn_id = state
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            send_packet(RoundezvousFromServer::Code { code }, &conn).await?;
            state
                .awaiting_exchange
                .write()
                .await
                .insert(code, (conn, socket_addr, conn_id));

            tracing::info!("new connection {conn_id}, sender announced itself");

            // spawn another task to close the connection after the timeout
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_secs(MAX_CONNECTION_AGE)).await;
                let conn = state.awaiting_exchange.write().await.remove(&code);
                if let Some((conn, _, conn_id)) = conn {
                    tracing::info!("closing connection {conn_id} due to timeout");
                    conn.close(0u32.into(), b"timeout");
                }
            });
        }
        RoundezvousToServer::Connect {
            version,
            code,
            socket_addr,
        } => {
            validate_version(version, &conn).await?;

            let (sender_conn, sender_addr, conn_id) = state
                .awaiting_exchange
                .write()
                .await
                .remove(&code)
                .ok_or(AppError::InvalidCode(code))?;

            send_packet(
                RoundezvousFromServer::SocketAddr { socket_addr },
                &sender_conn,
            )
            .await?;
            send_packet(
                RoundezvousFromServer::SocketAddr {
                    socket_addr: sender_addr,
                },
                &conn,
            )
            .await?;

            conn.closed().await;
            sender_conn.closed().await;

            tracing::info!("exchange complete for connection {conn_id}");
        }
    }

    Ok(())
}

async fn validate_version(version: Version, conn: &Connection) -> Result<(), AppError> {
    if !version.matches_major(&Version::from(VERSION)) {
        send_packet(
            RoundezvousFromServer::WrongVersion {
                expected: VERSION.to_string(),
            },
            conn,
        )
        .await?;
        Err(AppError::WrongVersion(Version::from(VERSION), version))
    } else {
        Ok(())
    }
}

/// Generate a session code
fn generate_code() -> [u8; CODE_LEN] {
    let mut code = [0; CODE_LEN];
    for i in code.iter_mut().take(CODE_LEN) {
        *i = CODE_CHARS[rand::random::<usize>() % CODE_CHARS.len()];
    }
    code
}

fn server_config() -> ServerConfig {
    let (cert, key) = self_signed_cert().expect("failed to generate self signed cert");

    let mut transport_config = quinn::TransportConfig::default();
    transport_config.keep_alive_interval(Some(time::Duration::from_secs(KEEP_ALIVE_INTERVAL_SECS)));

    ServerConfig::with_single_cert(vec![cert], key)
        .unwrap()
        .transport_config(Arc::new(transport_config))
        .to_owned()
}
