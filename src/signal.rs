use log::info;
use std::{
    ffi::CStr,
    fmt::{self, Display},
    os::raw::c_int,
};
use tokio::signal::unix::{self, SignalKind};

struct Signal(c_int);

impl Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    unix::signal(signal)
        .expect("Failed to install signal handler")
        .recv()
        .await;

    Signal(signal.as_raw_value())
}

pub async fn shutdown_signal() {
    let signal = tokio::select! {
        signal = wait_for_signal(SignalKind::interrupt()) => signal,
        signal = wait_for_signal(SignalKind::terminate()) => signal,
    };

    info!("Received {signal}");
}
