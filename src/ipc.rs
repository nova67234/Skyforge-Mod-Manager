//! Single-instance handling. When a second copy of the app starts (Windows launches one
//! for every `nxm://` link and every file opened with the app), it hands its arguments to
//! the copy already running over a loopback socket and quits.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

const PORT: u16 = 47121;

fn addr() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, PORT))
}

/// Sends `args` to a running instance. Returns false when there isn't one.
pub fn forward(args: &[String]) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr(), Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let payload = format!("skyrim-mod-manager\n{}", args.join("\n"));
    stream.write_all(payload.as_bytes()).is_ok() && stream.flush().is_ok()
}

pub struct Listener(TcpListener);

impl Listener {
    /// Claims the port; `None` means another instance owns it (or it's in use).
    pub fn bind() -> Option<Self> {
        TcpListener::bind(addr()).ok().map(Self)
    }

    /// Runs forever on its own thread, calling `on_args` with each batch of arguments.
    pub fn run(self, on_args: impl Fn(Vec<String>) + Send + 'static) {
        std::thread::spawn(move || {
            for stream in self.0.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = String::new();
                if stream.read_to_string(&mut buf).is_err() {
                    continue;
                }
                let mut lines = buf.lines();
                if lines.next() != Some("skyrim-mod-manager") {
                    continue;
                }
                on_args(lines.filter(|l| !l.is_empty()).map(str::to_string).collect());
            }
        });
    }
}
