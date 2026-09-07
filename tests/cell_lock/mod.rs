use super::*;

fn setup() -> (CellLock, At, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let lock = CellLock::new(
        dir.path().join("cell-lock.json"),
        Arc::new(Store::new(true)),
    );
    (lock, At::start(true, vec![]).unwrap(), dir)
}
fn params(mode: &str) -> Params {
    Params::parse("", &format!("action=lock_nr_manual&pci=0&earfcn=633984&scs=30&band=78&persistence={mode}&auto_unlock=1")).unwrap()
}
#[tokio::test]
async fn temporary_is_memory_only_and_persistent_restores_pci_zero() {
    let (lock, at, dir) = setup();
    lock.apply(&at, &params("temporary")).await.unwrap();
    assert!(!lock.path.exists());
    lock.apply(&at, &params("persistent")).await.unwrap();
    assert_eq!(lock.snapshot()["radios"][1]["persistent"], true);
    let reboot = CellLock::new(
        dir.path().join("cell-lock.json"),
        Arc::new(Store::new(true)),
    );
    reboot.restore(&at).await;
    assert!(
        at.trace
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("common/5g\",0,633984,30,78")
    );
    assert!(reboot.state.lock().unwrap().runtime[1].deadline.is_some());
    let raw = "+QNWLOCK: \"common/5g\",0,633984,30,78\nOK";
    assert_eq!(parser::network(raw)["cellLockStatus"], "已锁定5G");
}
#[tokio::test]
async fn timeout_disables_reboot_restore_and_keeps_values_for_manual_retry() {
    let (lock, at, dir) = setup();
    lock.apply(&at, &params("persistent")).await.unwrap();
    lock.state.lock().unwrap().runtime[1].deadline = Some(Instant::now() - Duration::from_secs(1));
    lock.check(&at).await;
    assert_eq!(lock.snapshot()["radios"][1]["phase"], "fallback");
    assert_eq!(lock.snapshot()["radios"][1]["persistent"], false);
    assert_eq!(lock.snapshot()["radios"][1]["values"][1], 633984);
    let reboot = CellLock::new(
        dir.path().join("cell-lock.json"),
        Arc::new(Store::new(true)),
    );
    at.trace.lock().unwrap().clear();
    reboot.restore(&at).await;
    assert!(at.trace.lock().unwrap().is_empty());
}
#[tokio::test]
async fn replacing_with_temporary_or_unlock_removes_boot_rule() {
    let (lock, at, _dir) = setup();
    lock.apply(&at, &params("persistent")).await.unwrap();
    lock.apply(&at, &params("temporary")).await.unwrap();
    assert_eq!(lock.snapshot()["radios"][1]["persistent"], false);
    assert!(lock.state.lock().unwrap().runtime[1].deadline.is_none());
    lock.apply(&at, &params("persistent")).await.unwrap();
    lock.apply(&at, &Params::parse("", "action=unlock_nr").unwrap())
        .await
        .unwrap();
    assert!(lock.state.lock().unwrap().settings.rules[1].is_none());
}
#[test]
fn dial_detection_rejects_empty_stale_and_link_local_addresses() {
    for ip in [
        "0.0.0.0",
        "127.0.0.1",
        "169.254.1.2",
        "::",
        "::1",
        "fe80::1",
        "bad",
    ] {
        assert!(!dialed(&format!(
            "+QMAP: \"WWAN\",1,1,\"IPV6\",\"{ip}\"\nOK"
        )));
    }
    assert!(dialed("+QMAP: \"WWAN\",1,1,\"IPV4\",\"10.1.2.3\"\nOK"));
    assert!(dialed("+QMAP: \"WWAN\",1,1,\"IPV6\",\"2408::123\"\nOK"));
    assert!(!dialed("+QMAP: \"WWAN\",1,1,\"IPV4\",\"10.1.2.3\"\nERROR"));
}
#[test]
fn invalid_saved_rule_is_never_executed() {
    let (lock, _at, _dir) = setup();
    std::fs::write(
        &lock.path,
        r#"{"rules":[null,{"values":[0,633984,17,78],"auto_unlock":true,"enabled":true}]}"#,
    )
    .unwrap();
    let loaded = CellLock::new(lock.path, Arc::new(Store::new(true)));
    assert!(loaded.state.lock().unwrap().settings.rules[1].is_none());
    assert!(
        !loaded.snapshot()["radios"][1]["error"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn data_session_keeps_rule_and_cancels_guard_after_settling() {
    let (lock, at, _dir) = setup();
    at.overrides.lock().unwrap().insert(
        "AT+QMAP=\"WWAN\"".into(),
        "+QMAP: \"WWAN\",1,1,\"IPV4\",\"10.1.2.3\"\nOK".into(),
    );
    lock.apply(&at, &params("persistent")).await.unwrap();
    lock.check(&at).await;
    assert!(lock.state.lock().unwrap().runtime[1].deadline.is_some());
    lock.state.lock().unwrap().runtime[1].deadline =
        Some(Instant::now() + Duration::from_secs(150));
    lock.check(&at).await;
    assert_eq!(lock.snapshot()["radios"][1]["phase"], "connected");
    assert_eq!(lock.snapshot()["radios"][1]["persistent"], true);
    assert!(lock.state.lock().unwrap().runtime[1].deadline.is_none());
}

#[tokio::test]
async fn rejected_lock_is_not_saved_and_failed_unlock_retries_even_after_connecting() {
    let (lock, at, _dir) = setup();
    at.overrides.lock().unwrap().insert(
        actions::network(&params("persistent")).unwrap(),
        "ERROR".into(),
    );
    assert!(lock.apply(&at, &params("persistent")).await.is_err());
    assert!(!lock.path.exists());
    at.overrides.lock().unwrap().clear();
    lock.apply(&at, &params("persistent")).await.unwrap();
    lock.state.lock().unwrap().runtime[1].deadline = Some(Instant::now() - Duration::from_secs(1));
    at.overrides
        .lock()
        .unwrap()
        .insert("AT+QNWLOCK=\"common/5g\",0".into(), "ERROR".into());
    lock.check(&at).await;
    assert_eq!(lock.snapshot()["radios"][1]["phase"], "fallback_error");
    assert_eq!(lock.snapshot()["radios"][1]["persistent"], false);
    at.overrides.lock().unwrap().clear();
    at.overrides.lock().unwrap().insert(
        "AT+QMAP=\"WWAN\"".into(),
        "+QMAP: \"WWAN\",1,1,\"IPV4\",\"10.1.2.3\"\nOK".into(),
    );
    lock.check(&at).await;
    assert_eq!(lock.snapshot()["radios"][1]["phase"], "fallback");
}

#[tokio::test]
async fn guard_is_optional_and_user_unlock_cannot_be_restored_by_startup() {
    let (lock, at, _dir) = setup();
    let mut p = params("persistent");
    p.0.retain(|(k, _)| k != "auto_unlock");
    lock.apply(&at, &p).await.unwrap();
    at.trace.lock().unwrap().clear();
    lock.check(&at).await;
    assert!(at.trace.lock().unwrap().is_empty());
    lock.apply(&at, &Params::parse("", "action=unlock_nr").unwrap())
        .await
        .unwrap();
    at.trace.lock().unwrap().clear();
    lock.restore(&at).await;
    assert!(at.trace.lock().unwrap().is_empty());
}
