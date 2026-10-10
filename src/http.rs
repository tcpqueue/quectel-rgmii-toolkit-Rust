use std::{sync::Arc, time::Duration};

pub fn builder() -> hyper::server::conn::http1::Builder {
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10))
        .max_buf_size(16 * 1024);
    builder
}

/// Tags each request with the client address so handlers can rate-limit password attempts.
pub fn with_peer(
    router: axum::Router,
    peer: std::net::SocketAddr,
) -> impl tower::Service<
    axum::http::Request<hyper::body::Incoming>,
    Response = axum::response::Response,
    Error = std::convert::Infallible,
    Future = impl Send,
> + Clone {
    use tower::ServiceExt;
    router.map_request(
        move |mut request: axum::http::Request<hyper::body::Incoming>| {
            request
                .extensions_mut()
                .insert(axum::extract::ConnectInfo(peer));
            request
        },
    )
}
pub async fn serve(listener: tokio::net::TcpListener, router: axum::Router) -> std::io::Result<()> {
    serve_with_permits(listener, router, Arc::new(tokio::sync::Semaphore::new(32))).await
}
/// Waits for the next connection. Accept errors such as EMFILE or ECONNABORTED are
/// transient, so they pause briefly instead of ending the server.
pub async fn accept(
    listener: &tokio::net::TcpListener,
) -> (tokio::net::TcpStream, std::net::SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(connection) => return connection,
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
pub async fn serve_with_permits(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    permits: Arc<tokio::sync::Semaphore>,
) -> std::io::Result<()> {
    loop {
        let (stream, peer) = accept(&listener).await;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let service =
            hyper_util::service::TowerToHyperService::new(with_peer(router.clone(), peer));
        tokio::spawn(async move {
            let _permit = permit;
            let _ = builder()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}
