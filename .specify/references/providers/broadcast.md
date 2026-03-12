# Broadcast

**When Required**: When the handler sends data to WebSocket clients (replies, push notifications, protocol messages).

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `Broadcast` trait provides WebSocket message sending via `Broadcast::send(provider, channel, data, sockets)`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl Broadcast for Provider {}
```

Usage in domain functions:

```rust
let payload = serde_json::to_vec(&response)?;
Broadcast::send(provider, "default", &payload, Some(vec![socket_id])).await?;
```

---

## MockProvider Implementation

### Basic Implementation

```rust
use omnia_sdk::Broadcast;

impl Broadcast for MockProvider {
    async fn send(
        &self,
        _name: &str,
        _data: &[u8],
        _sockets: Option<Vec<String>>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
```

### With Capture

Capture sent WebSocket messages for assertion in tests:

```rust
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct WsSent {
    pub channel: String,
    pub data: Vec<u8>,
    pub targets: Option<Vec<String>>,
}

pub struct MockProvider {
    pub ws_sent: Mutex<Vec<WsSent>>,
}

impl Broadcast for MockProvider {
    async fn send(
        &self,
        name: &str,
        data: &[u8],
        sockets: Option<Vec<String>>,
    ) -> anyhow::Result<()> {
        self.ws_sent.lock().unwrap().push(WsSent {
            channel: name.to_string(),
            data: data.to_vec(),
            targets: sockets,
        });
        Ok(())
    }
}
```

Usage in tests:

```rust
let provider = MockProvider {
    ws_sent: Mutex::new(Vec::new()),
    // ... other fields ...
};

// ... invoke handler ...

let sent = provider.ws_sent.lock().unwrap();
assert_eq!(sent.len(), 1);
assert_eq!(sent[0].channel, "default");
let reply: PongResponse = serde_json::from_slice(&sent[0].data).unwrap();
assert_eq!(reply.status, "ok");
```

## References

- [capabilities.md](../capabilities.md) -- Broadcast trait definition
- [guest-patterns.md](../guest-patterns.md) -- WebSocket guest export patterns
- [README.md](README.md) -- Provider composition rules
