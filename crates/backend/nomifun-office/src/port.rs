use std::net::TcpListener;
use std::time::Duration;

use crate::error::OfficeError;

const CONNECT_TIMEOUT_MS: u64 = 100;

pub fn allocate_port() -> Result<u16, OfficeError> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

pub async fn is_port_listening(port: u16) -> bool {
    match tokio::time::timeout(
        Duration::from_millis(CONNECT_TIMEOUT_MS),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    {
        Ok(result) => result.is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_port_returns_nonzero() {
        let port = allocate_port().unwrap();
        assert!(port > 0);
    }

    #[test]
    fn allocate_port_returns_different_ports() {
        let p1 = allocate_port().unwrap();
        let p2 = allocate_port().unwrap();
        assert_ne!(p1, p2);
    }

    #[tokio::test]
    async fn is_port_listening_false_for_unused_port() {
        let port = allocate_port().unwrap();
        assert!(!is_port_listening(port).await);
    }

    #[tokio::test]
    async fn is_port_listening_true_for_active_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_port_listening(port).await);
    }
}
