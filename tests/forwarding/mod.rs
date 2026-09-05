use super::*;
fn message(index: u64, text: &str) -> Value {
    json!({"sender":"10086","date":"26/09/06,12:00:00+32","text":text,"indices":[index]})
}
fn notification() -> Notification {
    Notification {
        id: "sample".into(),
        device: "Test module".into(),
        sender: "10086".into(),
        received_at: "2026-09-06T12:00:00+08:00".into(),
        text: "Test SMS".into(),
        parts: Vec::new(),
    }
}
fn forwarder() -> (Forwarder, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (
        Forwarder::new(
            dir.path().join("forwarding.json"),
            Arc::new(Store::new(true)),
        ),
        dir,
    )
}
#[test]
fn baseline_dedup_and_recycled_storage() {
    let mut d = Detector::default();
    let old = message(1, "old");
    let next = message(2, "new");
    assert!(d.scan(vec![old.clone()], "device").unwrap().is_empty());
    assert_eq!(
        d.scan(vec![old.clone(), next.clone()], "device")
            .unwrap()
            .len(),
        1
    );
    assert!(
        d.scan(vec![old.clone(), next], "device")
            .unwrap()
            .is_empty()
    );
    assert!(d.scan(vec![], "device").unwrap().is_empty());
    assert_eq!(
        d.scan(vec![message(1, "another")], "device").unwrap().len(),
        1
    );
    assert_eq!(d.seen.len(), 1);
}
#[test]
fn multipart_waits_for_all_parts_and_never_replays_baseline_fragments() {
    let mut d = Detector::default();
    d.scan(vec![], "device").unwrap();
    let mut first = message(1, "hello ");
    first["concatRef"] = json!("10086:12");
    first["concatTotal"] = json!(2);
    first["concatSeq"] = json!(1);
    let mut second = message(2, "world");
    second["concatRef"] = json!("10086:12");
    second["concatTotal"] = json!(2);
    second["concatSeq"] = json!(2);
    assert!(d.scan(vec![first.clone()], "device").unwrap().is_empty());
    let notes = d
        .scan(vec![second.clone(), first.clone()], "device")
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "hello world");
    assert!(
        d.scan(vec![second.clone(), first.clone()], "device")
            .unwrap()
            .is_empty()
    );
    let mut d = Detector::default();
    d.scan(vec![first.clone()], "device").unwrap();
    assert!(d.scan(vec![first, second], "device").unwrap().is_empty());
}
#[test]
fn multipart_reference_reuse_does_not_join_separate_messages() {
    let mut d = Detector::default();
    d.scan(vec![], "device").unwrap();
    let mut entries = Vec::new();
    for i in 0..4 {
        let mut v = message(i + 1, if i < 2 { "A" } else { "B" });
        v["concatRef"] = json!("same-ref");
        v["concatTotal"] = json!(2);
        v["concatSeq"] = json!(i % 2 + 1);
        entries.push(v);
    }
    let mut result = d
        .scan(entries, "device")
        .unwrap()
        .iter()
        .map(|n| n.text.clone())
        .collect::<Vec<_>>();
    result.sort();
    assert_eq!(result, vec!["AA", "BB"]);
}
#[test]
fn only_received_sms_enters_forwarding() {
    let raw = "+CMGL: 1,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nhello\n+CMGL: 2,\"STO SENT\",\"10086\",,\"26/09/06,12:00:01+32\"\noutgoing\nOK";
    let entries = sms::received(raw);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["text"], "hello");
    assert!(sms::received("+CMGL: 3,0,,12\ninvalid PDU\nOK").is_empty());
}
#[tokio::test]
async fn received_sms_from_at_transaction_keeps_full_envelope() {
    let at = At::start(true, vec![]).unwrap();
    at.overrides.lock().unwrap().insert(
        "sms".into(),
        "+CMGL: 2,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nNew SMS: 123456\nOK".into(),
    );
    let raw = at.page("sms", true).await.unwrap();
    assert!(parser::ok(&raw));
    let entries = sms::received(&raw);
    assert_eq!(entries.len(), 1, "{raw}");
    assert_eq!(entries[0]["text"], "New SMS: 123456");
    let mut d = Detector::default();
    d.scan(entries, "Test").unwrap();
    at.overrides.lock().unwrap().insert(
        "sms".into(),
        "+CMGL: 3,\"REC READ\",\"10086\",,\"26/09/06,12:00:00+32\"\nNext SMS\nOK".into(),
    );
    at.invalidate().await;
    let raw = at.page("sms", false).await.unwrap();
    assert_eq!(d.scan(sms::received(&raw), "Test").unwrap().len(), 1);
}
#[test]
fn utf8_chunks_preserve_entire_message() {
    let text = "短信😀".repeat(1500);
    let parts = chunks(&text, 1200);
    assert!(parts.len() > 1);
    assert!(parts.iter().all(|p| p.len() <= 1200));
    assert_eq!(parts.concat(), text);
}
#[test]
fn platform_payloads_and_signatures() {
    let n = notification();
    let server = Channel {
        platform: "serverchan".into(),
        token: "SCT123456789".into(),
        ..Channel::default()
    };
    let (url, p) = payload(&server, &n, &n.text, 0, 1, 1700000000).unwrap();
    assert_eq!(url.as_str(), "https://sctapi.ftqq.com/SCT123456789.send");
    assert!(p["desp"].as_str().unwrap().contains("10086"));
    for platform in ["wecom", "dingtalk", "feishu", "webhook"] {
        let c = Channel {
            platform: platform.into(),
            url: "https://example.com/hook?token=keep".into(),
            secret: "secret".into(),
            ..Channel::default()
        };
        let (url, p) = payload(&c, &n, &n.text, 0, 1, 1700000000).unwrap();
        if platform == "dingtalk" {
            let pairs: HashMap<_, _> = url.query_pairs().collect();
            assert_eq!(pairs["timestamp"], "1700000000000");
            assert_eq!(pairs["sign"], sign("secret", "1700000000000\nsecret"));
            assert_eq!(pairs["token"], "keep");
        }
        if platform == "feishu" {
            assert_eq!(p["timestamp"], "1700000000");
            assert_eq!(p["sign"], sign("1700000000\nsecret", ""));
        }
        if platform == "webhook" {
            assert_eq!(p["text"], "Test SMS");
            assert_eq!(p["sender"], "10086");
            assert_eq!(p["parts"], 1);
        }
    }
    assert_eq!(
        sign("key", "The quick brown fox jumps over the lazy dog"),
        "97yD9DBThCSxMpjmqm+xQ+9NWaFJRhdZl0edvC0aPNg="
    );
}
#[tokio::test]
async fn settings_persist_privately_preserve_secrets_and_skip_identical_writes() {
    let (f, _dir) = forwarder();
    let mut settings = Settings::default();
    settings.channels[0].token = "SCT123456789".into();
    settings.channels[0].enabled = true;
    f.save(settings.clone()).await.unwrap();
    let first = std::fs::metadata(&f.path).unwrap().modified().unwrap();
    let snapshot = f.snapshot().to_string();
    assert!(!snapshot.contains("SCT123456789"));
    assert!(snapshot.contains("has_token"));
    settings.channels[0].token.clear();
    f.save(settings.clone()).await.unwrap();
    assert_eq!(
        first,
        std::fs::metadata(&f.path).unwrap().modified().unwrap()
    );
    let loaded = Forwarder::new(f.path.clone(), Arc::new(Store::new(true)));
    assert_eq!(
        loaded.state.lock().unwrap().settings.channels[0].token,
        "SCT123456789"
    );
    settings.channels[0].clear = true;
    f.save(settings).await.unwrap();
    assert_eq!(f.snapshot()["channels"][0]["has_token"], false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn expired_jobs_are_not_sent_and_disabling_clears_queue() {
    let (f, _dir) = forwarder();
    {
        let mut s = f.state.lock().unwrap();
        s.queue.push_back(Job {
            notification: Arc::new(notification()),
            channel: 4,
            part: 0,
            attempts: 1,
            created: Instant::now() - Duration::from_secs(181),
            next: Instant::now(),
        });
    }
    f.deliver_next().await;
    assert_eq!(f.snapshot()["queued"], 0);
    assert_eq!(f.snapshot()["records"][0]["detail"], "retry window expired");
    assert!(f.client.get().is_none());
}
#[test]
fn rejects_credential_leaks_and_unbounded_configuration() {
    let mut s = Settings::default();
    s.channels[1].enabled = true;
    s.channels[1].url = "https://evil.example/cgi-bin/webhook/send?key=secret".into();
    assert!(s.validate().is_err());
    s.channels[1].url = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=test".into();
    assert!(s.validate().is_ok());
    s.channels[4].enabled = true;
    s.channels[4].url = "http://user:password@localhost/hook".into();
    assert!(s.validate().is_err());
    s.channels[4].url = "http://127.0.0.1/hook".into();
    assert!(s.validate().is_ok());
    s.channels[4].token = "Bearer test\r\nHost: other".into();
    assert!(s.validate().is_err());
}
#[test]
fn cleanup_requires_every_channel_to_confirm() {
    let (f, _dir) = forwarder();
    let mut n = notification();
    n.parts = vec![Part {
        index: 1,
        fingerprint: [1; 16],
    }];
    let mut s = f.state.lock().unwrap();
    s.settings.delete_after_day = true;
    s.outcomes.insert(n.id.clone(), (2, false));
    f.completed(&mut s, &n, true);
    assert_eq!(f.cleanup.status()["pending"], 0);
    f.completed(&mut s, &n, false);
    assert_eq!(f.cleanup.status()["pending"], 0);
    s.outcomes.insert(n.id.clone(), (2, false));
    f.completed(&mut s, &n, true);
    f.completed(&mut s, &n, true);
    assert_eq!(f.cleanup.status()["pending"], 1);
}
#[tokio::test]
async fn disabling_sms_clears_pending_work_and_preserves_channel_settings() {
    let (f, _dir) = forwarder();
    let mut settings = Settings::default();
    settings.channels[0].token = "SCT123456789".into();
    settings.channels[0].enabled = true;
    settings.enabled = true;
    f.save(settings).await.unwrap();
    {
        let mut s = f.state.lock().unwrap();
        s.queue.push_back(Job {
            notification: Arc::new(notification()),
            channel: 0,
            part: 0,
            attempts: 0,
            created: Instant::now(),
            next: Instant::now(),
        });
    }
    f.sms_settings(true, true).await.unwrap();
    assert_eq!(
        f.snapshot()["queued"],
        1,
        "changing cleanup policy must preserve retries"
    );
    f.sms_settings(false, true).await.unwrap();
    assert!(!f.sms_enabled());
    assert_eq!(f.snapshot()["queued"], 0);
    assert_eq!(f.snapshot()["channels"][0]["has_token"], true);
    let loaded = Forwarder::new(f.path.clone(), Arc::new(Store::new(true)));
    assert!(!loaded.sms_enabled());
    assert_eq!(loaded.snapshot()["ready"], false);
}
#[tokio::test]
async fn webhook_posts_json_and_requires_http_success() {
    use axum::{Router, routing::post};
    let received = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = received.clone();
    let app = Router::new().route(
        "/hook",
        post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer sample");
                assert_eq!(headers["idempotency-key"], "sample-1");
                capture.lock().unwrap().push(body);
                axum::http::StatusCode::NO_CONTENT
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (f, _dir) = forwarder();
    let c = Channel {
        platform: "webhook".into(),
        url: format!("http://{address}/hook"),
        token: "Bearer sample".into(),
        ..Channel::default()
    };
    let n = notification();
    f.send(&c, &n, &n.text, 0, 1, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(received.lock().unwrap()[0]["device"], "Test module");
    let bad = Channel {
        url: format!("http://{address}/missing"),
        ..c
    };
    assert_eq!(
        f.send(&bad, &n, &n.text, 0, 1, Duration::from_secs(5))
            .await
            .unwrap_err(),
        ("HTTP 404".into(), false)
    );
    server.abort();
}
