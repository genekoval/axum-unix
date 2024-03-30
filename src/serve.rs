use crate::endpoint::{Endpoint, UnixDomainSocket};

use axum::{extract::Request, Router};
use futures_util::{pin_mut, FutureExt};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use log::{error, info, log_enabled, trace, warn, Level::Trace};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::{
    net::{unix, TcpListener, UnixListener},
    task::JoinHandle,
};
use tokio_util::{net::Listener, sync::CancellationToken, task::TaskTracker};
use tower::Service;

struct PathGuard(PathBuf);

impl Drop for PathGuard {
    fn drop(&mut self) {
        let path = self.0.as_path();

        match std::fs::remove_file(path) {
            Ok(()) => {
                trace!("Removed Unix domain socket file '{}'", path.display())
            }
            Err(err) => warn!(
                "Failed to remove Unix domain socket file '{}': {err}",
                path.display()
            ),
        }
    }
}

trait DisplayAddr {
    fn addr_string(&self) -> Option<String>;
}

impl DisplayAddr for std::net::SocketAddr {
    fn addr_string(&self) -> Option<String> {
        Some(self.to_string())
    }
}

impl DisplayAddr for unix::SocketAddr {
    fn addr_string(&self) -> Option<String> {
        None
    }
}

async fn listen<T>(mut listener: T, app: Router, token: CancellationToken)
where
    T: Listener,
    T::Addr: DisplayAddr,
    T::Io: Send + Unpin + 'static,
{
    let tracker = TaskTracker::new();

    loop {
        let (socket, remote) = tokio::select! {
            connection = listener.accept() => {
                match connection {
                    Ok(connection) => connection,
                    Err(err) => {
                        error!("Failed to accept connection: {err}");
                        continue;
                    }
                }
            },
            _ = token.cancelled() => {
                trace!("Signal received, not accepting new connections");
                break;
            }
        };

        match remote.addr_string() {
            Some(addr) => trace!("Connection accepted from {addr}"),
            None => trace!("Connection accepted"),
        };

        let socket = TokioIo::new(socket);
        let tower_service = app.clone();
        let cloned_token = token.clone();

        tracker.spawn(async move {
            let hyper_service =
                service_fn(move |request: Request<Incoming>| {
                    tower_service.clone().call(request)
                });

            let builder = Builder::new(TokioExecutor::new());
            let connection =
                builder.serve_connection_with_upgrades(socket, hyper_service);
            pin_mut!(connection);

            let cancellation = cloned_token.cancelled().fuse();
            pin_mut!(cancellation);

            loop {
                tokio::select! {
                    result = connection.as_mut() => {
                        if let Err(err) = result {
                            error!("Failed to serve connection: {err}");
                        }
                        break;
                    }
                    _ = &mut cancellation => {
                        trace!(
                            "Cancellation requested for connection task, \
                            starting graceful shutdown"
                        );
                        connection.as_mut().graceful_shutdown();
                    }
                }
            }

            trace!("Connection closed");
        });
    }

    drop(listener);

    if log_enabled!(Trace) {
        let tasks = tracker.len();

        if tasks > 0 {
            trace!(
                "Waiting for {tasks} task{} to finish",
                match tasks {
                    1 => "",
                    _ => "s",
                }
            );
        }
    }

    tracker.close();
    tracker.wait().await;
}

async fn serve_inet<F>(
    addr: &str,
    app: Router,
    token: CancellationToken,
    f: F,
) -> Result<JoinHandle<()>, String>
where
    F: FnOnce(Option<SocketAddr>),
{
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|err| format!("failed to bind to address '{addr}': {err}"))?;

    match listener.local_addr() {
        Ok(addr) => {
            info!("Listening for connections on {addr}");
            f(Some(addr));
        }
        Err(err) => {
            warn!("Could not retrieve TCP listener's local address: {err}");
            info!("Listening for connections on {addr}");
            f(None);
        }
    };

    let handle = tokio::spawn(async move {
        listen(listener, app, token).await;
    });

    Ok(handle)
}

async fn serve_unix<F>(
    uds: &UnixDomainSocket,
    app: Router,
    token: CancellationToken,
    f: F,
) -> Result<JoinHandle<()>, String>
where
    F: FnOnce(Option<SocketAddr>),
{
    let path = uds.path.as_path();
    let listener = UnixListener::bind(path).map_err(|err| {
        format!(
            "failed to bind to Unix domain socket path '{}': {err}",
            path.display()
        )
    })?;
    let guard = PathGuard(path.to_path_buf());

    uds.set_permissions()?;

    info!("Listening for connections on \"{}\"", path.display());
    f(None);

    let handle = tokio::spawn(async move {
        listen(listener, app, token).await;
        drop(guard);
    });

    Ok(handle)
}

pub async fn serve<F>(
    endpoint: &Endpoint,
    app: Router,
    token: CancellationToken,
    f: F,
) -> Result<JoinHandle<()>, String>
where
    F: FnOnce(Option<SocketAddr>),
{
    match endpoint {
        Endpoint::Inet(inet) => serve_inet(inet, app, token, f).await,
        Endpoint::Unix(unix) => serve_unix(unix, app, token, f).await,
    }
}
