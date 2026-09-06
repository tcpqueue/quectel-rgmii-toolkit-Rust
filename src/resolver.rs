use std::{
    net::{IpAddr, ToSocketAddrs},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

type Pending = (String, oneshot::Receiver<Option<IpAddr>>);

#[derive(Default)]
pub struct PingResolver {
    pending: Option<Pending>,
    cached: Option<(String, IpAddr, Instant)>,
    retry: Option<(String, Instant)>,
}

impl PingResolver {
    pub async fn resolve(
        &mut self,
        target: &str,
        deadline: tokio::time::Instant,
    ) -> Option<IpAddr> {
        if let Ok(ip) = target.parse() {
            return Some(ip);
        }
        if let Some((name, ip, until)) = &self.cached
            && name == target
            && *until > Instant::now()
        {
            return Some(*ip);
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
                .name("ping-dns".into())
                .stack_size(256 * 1024)
                .spawn(move || {
                    let ip = (name.as_str(), 0)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addrs| {
                            let first = addrs.next().map(|a| a.ip());
                            addrs.find(|a| a.is_ipv4()).map(|a| a.ip()).or(first)
                        });
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
        if let Some(ip) = ip {
            self.cached = Some((name.clone(), ip, Instant::now() + Duration::from_secs(60)));
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
        tx.send(Some(ip)).unwrap();
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
        tx.send(Some(ip)).unwrap();
        assert_eq!(resolver.resolve("host.test", deadline()).await, Some(ip));
        assert!(resolver.retry.is_none());
    }
}
