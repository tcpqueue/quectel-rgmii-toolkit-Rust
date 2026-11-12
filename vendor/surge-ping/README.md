# surge-ping 0.8.4

Source: https://crates.io/crates/surge-ping/0.8.4 (MIT).

Local changes in src/client.rs:

- Stop the receive task on socket errors and release pending reply waiters.
- Keep cloned clients usable until the last client is dropped.
- Serialize waiter registration with shutdown.

The application discards failed clients and recreates them on a later scheduled probe.
The receiver never retries an errored socket in a loop.

Formatting follows the workspace rustfmt; ICMP uses a derived Default for the workspace Clippy checks.

Upstream crate SHA-256: 30498e9c9feba213c3df6ed675bdf75519ccbee493517e7225305898c86cac05.
