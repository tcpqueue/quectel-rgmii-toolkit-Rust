use crate::{listen_address, server::App};
use anyhow::{Context, Result};
use std::sync::Arc;

pub async fn serve(app: Arc<App>) -> Result<()> {
    let cfg = &app.config;
    if !cfg.cert.exists() || !cfg.key.exists() {
        let (ca_key, ca_params) = if cfg.ca_cert.exists() && cfg.ca_key.exists() {
            (
                rcgen::KeyPair::from_pem(&std::fs::read_to_string(&cfg.ca_key)?)?,
                rcgen::CertificateParams::from_ca_cert_pem(&std::fs::read_to_string(
                    &cfg.ca_cert,
                )?)?,
            )
        } else {
            let key = rcgen::KeyPair::generate()?;
            let mut params = rcgen::CertificateParams::default();
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "ZBIMS Local CA");
            params.key_usages = vec![
                rcgen::KeyUsagePurpose::KeyCertSign,
                rcgen::KeyUsagePurpose::CrlSign,
                rcgen::KeyUsagePurpose::DigitalSignature,
            ];
            let certificate = params.clone().self_signed(&key)?;
            app.store
                .write(&cfg.ca_key, key.serialize_pem().as_bytes(), 0o600)?;
            app.store
                .write(&cfg.ca_cert, certificate.pem().as_bytes(), 0o644)?;
            (key, params)
        };
        let ca = ca_params.self_signed(&ca_key)?;
        let key = rcgen::KeyPair::generate()?;
        let mut params = rcgen::CertificateParams::new(vec![
            "localhost".into(),
            "zbims".into(),
            "zbims.local".into(),
            "127.0.0.1".into(),
            "::1".into(),
            "192.168.225.1".into(),
            "192.168.1.1".into(),
        ])?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "SimpleAdmin");
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let cert = params.signed_by(&key, &ca, &ca_key)?;
        app.store
            .write(&cfg.key, key.serialize_pem().as_bytes(), 0o600)?;
        app.store.write(&cfg.cert, cert.pem().as_bytes(), 0o644)?;
    }
    let cert = std::fs::read(&cfg.cert)?;
    let key = std::fs::read(&cfg.key)?;
    let certs =
        rustls_pemfile::certs(&mut cert.as_slice()).collect::<std::result::Result<Vec<_>, _>>()?;
    let key =
        rustls_pemfile::private_key(&mut key.as_slice())?.context("TLS private key missing")?;
    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind(listen_address(&cfg.https)).await?;
    let redirect = tokio::net::TcpListener::bind(listen_address(&cfg.http)).await?;
    let https_port = cfg.https.rsplit(':').next().unwrap_or("443").to_owned();
    tokio::spawn(async move {
        let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let port = https_port.clone();
            async move {
                let host = request
                    .headers()
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|h| h.parse::<axum::http::uri::Authority>().ok())
                    .map(|a| a.host().to_owned())
                    .unwrap_or_else(|| "192.168.225.1".into());
                let authority = if port == "443" {
                    host
                } else {
                    format!("{host}:{port}")
                };
                axum::response::Redirect::permanent(&format!(
                    "https://{authority}{}",
                    request.uri()
                ))
            }
        });
        let _ = axum::serve(redirect, router).await;
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let acceptor = acceptor.clone();
        let router = app.router();
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(Ok(stream)) =
                tokio::time::timeout(std::time::Duration::from_secs(10), acceptor.accept(stream))
                    .await
            {
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = hyper_util::service::TowerToHyperService::new(router);
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection_with_upgrades(io, service)
                .await;
            }
        });
    }
}
