mod endpoint;
mod serde;

pub use endpoint::{Endpoint, UnixDomainSocket};

use axum::{extract::Request, Router};
use futures_util::{pin_mut, FutureExt};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use log::{error, info, log_enabled, trace, warn, Level::Trace};
use std::net::SocketAddr;
use std::{ffi::CStr, fmt::Display, os::raw::c_int, sync::Arc};
use tokio::{
    io::AsyncRead, io::AsyncWrite, net, signal::unix::SignalKind, sync::watch,
};
use tower::Service;

trait Listener {
    async fn accept(
        &self,
    ) -> std::io::Result<(
        impl AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
        Option<SocketAddr>,
    )>;
}

struct TcpListener(net::TcpListener);

impl TcpListener {
    async fn bind(addr: &str) -> Result<Self, String> {
        let inner = net::TcpListener::bind(addr).await.map_err(|err| {
            format!("Failed to bind to address '{addr}': {err}")
        })?;

        Ok(Self(inner))
    }
}

impl Listener for TcpListener {
    async fn accept(
        &self,
    ) -> std::io::Result<(
        impl AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
        Option<SocketAddr>,
    )> {
        self.0
            .accept()
            .await
            .map(|(socket, remote)| (socket, Some(remote)))
    }
}

struct UnixListener<'a> {
    inner: net::UnixListener,
    socket: &'a UnixDomainSocket,
}

impl<'a> UnixListener<'a> {
    fn bind(socket: &'a UnixDomainSocket) -> Result<Self, String> {
        let path = socket.path.as_path();
        let inner = net::UnixListener::bind(path).map_err(|err| {
            format!(
                "Failed to bind Unix domain socket path '{}': {err}",
                path.display()
            )
        })?;

        // Construct Self before setting permissions so that its
        // Drop implementation executes even if setting permissions fails.
        let result = Self { inner, socket };
        socket.set_permissions()?;

        Ok(result)
    }
}

impl<'a> Listener for UnixListener<'a> {
    async fn accept(
        &self,
    ) -> std::io::Result<(
        impl AsyncRead + AsyncWrite + Send + Sync + Unpin + 'static,
        Option<SocketAddr>,
    )> {
        self.inner.accept().await.map(|(socket, _)| (socket, None))
    }
}

impl<'a> Drop for UnixListener<'a> {
    fn drop(&mut self) {
        self.socket.remove_file();
    }
}

async fn listen<T>(listener: T, app: Router)
where
    T: Listener,
{
    let signal = shutdown_signal();
    let (signal_tx, signal_rx) = watch::channel(());
    let signal_tx = Arc::new(signal_tx);
    tokio::spawn(async move {
        signal.await;
        drop(signal_rx);
    });

    let (tasks_tx, tasks_rx) = watch::channel(());

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
            _ = signal_tx.closed() => {
                trace!("Signal received, not accepting new connections");
                break;
            }
        };

        match remote {
            Some(addr) => trace!("Connection accepted from {addr}"),
            None => trace!("Connection accepted"),
        };

        let socket = TokioIo::new(socket);
        let tower_service = app.clone();

        let signal_tx = Arc::clone(&signal_tx);
        let tasks_rx = tasks_rx.clone();

        tokio::spawn(async move {
            let hyper_service =
                service_fn(move |request: Request<Incoming>| {
                    tower_service.clone().call(request)
                });

            let builder = Builder::new(TokioExecutor::new());
            let connection =
                builder.serve_connection_with_upgrades(socket, hyper_service);
            pin_mut!(connection);

            let signal_closed = signal_tx.closed().fuse();
            pin_mut!(signal_closed);

            loop {
                tokio::select! {
                    result = connection.as_mut() => {
                        if let Err(err) = result {
                            error!("Failed to serve connection: {err}");
                        }
                        break;
                    }
                    _ = &mut signal_closed => {
                        trace!(
                            "Signal received in task, \
                            starting graceful shutdown"
                        );
                        connection.as_mut().graceful_shutdown();
                    }
                }
            }

            trace!("Connection closed");
            drop(tasks_rx);
        });
    }

    drop(tasks_rx);
    drop(listener);

    if log_enabled!(Trace) {
        let tasks = tasks_tx.receiver_count();

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

    tasks_tx.closed().await;
}

async fn serve_inet<F>(addr: &str, app: Router, f: F) -> Result<(), String>
where
    F: FnOnce(Option<SocketAddr>),
{
    let listener = TcpListener::bind(addr).await?;

    match listener.0.local_addr() {
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

    listen(listener, app).await;

    Ok(())
}

async fn serve_unix<F>(
    uds: &UnixDomainSocket,
    app: Router,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(Option<SocketAddr>),
{
    let listener = UnixListener::bind(uds)?;

    info!("Listening for connections on \"{}\"", uds.path.display());
    f(None);

    listen(listener, app).await;

    Ok(())
}

pub async fn serve<F>(
    endpoint: &Endpoint,
    app: Router,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(Option<SocketAddr>),
{
    match endpoint {
        Endpoint::Inet(inet) => serve_inet(inet, app, f).await,
        Endpoint::Unix(unix) => serve_unix(unix, app, f).await,
    }
}

struct Signal(c_int);

impl Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "signal ({})", self.0)?;

        unsafe {
            let ptr = libc::strsignal(self.0);

            if ptr.is_null() {
                Ok(())
            } else {
                let string = CStr::from_ptr(ptr).to_str().unwrap();
                write!(f, ": {string}")
            }
        }
    }
}

async fn wait_for_signal(signal: SignalKind) -> Signal {
    tokio::signal::unix::signal(signal)
        .expect("Failed to install signal handler")
        .recv()
        .await;

    Signal(signal.as_raw_value())
}

async fn shutdown_signal() {
    let signal = tokio::select! {
        signal = wait_for_signal(SignalKind::interrupt()) => signal,
        signal = wait_for_signal(SignalKind::terminate()) => signal,
    };

    info!("Received {signal}");
}
