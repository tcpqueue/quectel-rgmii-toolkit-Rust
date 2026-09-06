use std::{
    net::{IpAddr, ToSocketAddrs},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

type Pending = (String, oneshot::Receiver<Option<Vec<IpAddr>>>);

#[derive(Clone, Default)]
pub struct HttpResolver(std::sync::Arc<tokio::sync::Mutex<PingResolver>>);

impl reqwest::dns::Resolve for HttpResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let state = self.0.clone();
        Box::pin(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
            let mut resolver = tokio::time::timeout_at(deadline, state.lock()).await?;
            let ips = resolver
                .resolve_all(name.as_str(), deadline)
                .await
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS unavailable")
                })?;
            let addrs: reqwest::dns::Addrs =
                Box::new(ips.into_iter().map(|ip| std::net::SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

#[derive(Default)]
pub struct PingResolver {
    pending: Option<Pending>,
    cached: Option<(String, Vec<IpAddr>, Instant)>,
    retry: Option<(String, Instant)>,
}

impl PingResolver {
    pub async fn resolve(
        &mut self,
        target: &str,
        deadline: tokio::time::Instant,
    ) -> Option<IpAddr> {
        let ips = self.resolve_all(target, deadline).await?;
        ips.iter().find(|ip| ip.is_ipv4()).or(ips.first()).copied()
    }
    async fn resolve_all(
        &mut self,
        target: &str,
        deadline: tokio::time::Instant,
    ) -> Option<Vec<IpAddr>> {
        if let Ok(ip) = target.parse() {
            return Some(vec![ip]);
        }
        if let Some((name, ip, until)) = &self.cached
            && name == target
            && *until > Instant::now()
        {
            return Some(ip.clone());
        }
        if self.pending.is_none() {
            if let Some((name, until)) = &self.retry
                && name == target
                && *until > Instant::now()
            {
                return None;
            }
            let name = target.to_owned();
            let (tx, rx) = oneshot::channel();
            // libc DNS calls cannot be cancelled by an async timeout. Keep at most
            // one outstanding lookup, outside Tokio's shared file-I/O pool.
            if std::thread::Builder::new()
                .name("network-dns".into())
                .stack_size(256 * 1024)
                .spawn(move || {
                    let ip = (name.as_str(), 0)
                        .to_socket_addrs()
                        .ok()
                        .map(|addrs| addrs.take(16).map(|a| a.ip()).collect::<Vec<_>>())
                        .filter(|ips| !ips.is_empty());
                    let _ = tx.send(ip);
                })
                .is_err()
            {
                self.retry = Some((target.into(), Instant::now() + Duration::from_secs(5)));
                return None;
            }
            self.pending = Some((target.into(), rx));
        }
        let (name, rx) = self.pending.as_mut()?;
        // Retain the receiver on timeout, including across target changes.
        // Dropping it and launching another lookup would recreate the leak.
        let result = tokio::time::timeout_at(deadline, rx).await.ok()?;
        let name = name.clone();
        self.pending = None;
        let ip = result.ok().flatten();
        if let Some(ref ip) = ip {
            self.cached = Some((
                name.clone(),
                ip.clone(),
                Instant::now() + Duration::from_secs(60),
            ));
            self.retry = None;
        } else {
            self.retry = Some((name.clone(), Instant::now() + Duration::from_secs(5)));
        }
        if name == target { ip } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_http_requests_keep_the_same_lookup() {
        use reqwest::dns::Resolve;
        let (tx, rx) = oneshot::channel();
        let resolver = HttpResolver::default();
        resolver.0.lock().await.pending = Some(("notify.test".into(), rx));
        for _ in 0..3 {
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(5),
                    resolver.resolve("notify.test".parse().unwrap())
                )
                .await
                .is_err()
            );
            assert!(resolver.0.lock().await.pending.is_some());
        }
        tx.send(Some(vec![
            "192.0.2.1".parse().unwrap(),
            "2001:db8::1".parse().unwrap(),
        ]))
        .unwrap();
        let mut addresses = resolver
            .resolve("notify.test".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(addresses.next().unwrap(), "192.0.2.1:0".parse().unwrap());
        assert_eq!(
            addresses.next().unwrap(),
            "[2001:db8::1]:0".parse().unwrap()
        );
    }
    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_millis(10)
    }
    #[tokio::test]
    async fn timeout_retains_one_lookup_and_discards_old_target_result() {
        let (tx, rx) = oneshot::channel();
        let mut resolver = PingResolver {
            pending: Some(("old.test".into(), rx)),
            ..Default::default()
        };
        for _ in 0..3 {
            assert_eq!(resolver.resolve("new.test", deadline()).await, None);
            assert_eq!(resolver.pending.as_ref().unwrap().0, "old.test");
        }
        let ip = "192.0.2.1".parse().unwrap();
        assert_eq!(resolver.resolve("192.0.2.1", deadline()).await, Some(ip));
        tx.send(Some(vec![ip])).unwrap();
        assert_eq!(resolver.resolve("new.test", deadline()).await, None);
        assert!(resolver.pending.is_none());
        assert_eq!(resolver.resolve("old.test", deadline()).await, Some(ip));
    }
    #[tokio::test]
    async fn failed_query_backs_off_then_allows_recovery() {
        let (tx, rx) = oneshot::channel();
        let mut resolver = PingResolver {
            pending: Some(("host.test".into(), rx)),
            ..Default::default()
        };
        tx.send(None).unwrap();
        assert_eq!(resolver.resolve("host.test", deadline()).await, None);
        assert_eq!(resolver.resolve("host.test", deadline()).await, None);
        assert!(resolver.pending.is_none());
        let (tx, rx) = oneshot::channel();
        resolver.pending = Some(("host.test".into(), rx));
        let ip = "192.0.2.2".parse().unwrap();
        tx.send(Some(vec![ip])).unwrap();
        assert_eq!(resolver.resolve("host.test", deadline()).await, Some(ip));
        assert!(resolver.retry.is_none());
    }
}
