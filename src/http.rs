use std::{sync::Arc, time::Duration};

pub fn builder() -> hyper::server::conn::http1::Builder {
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10))
        .max_buf_size(16 * 1024);
    builder
}

pub async fn serve(listener: tokio::net::TcpListener, router: axum::Router) -> std::io::Result<()> {
    serve_with_permits(listener, router, Arc::new(tokio::sync::Semaphore::new(32))).await
}
pub async fn serve_with_permits(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    permits: Arc<tokio::sync::Semaphore>,
) -> std::io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let service = hyper_util::service::TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let _ = builder()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}
