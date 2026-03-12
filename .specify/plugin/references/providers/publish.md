# Publish

**When Required**: Component publishes messages/events.

For the trait definition and method signatures, see [capabilities.md](../capabilities.md). For provider composition rules, see [README.md](README.md).

---

## Production Patterns

The `Publish` trait provides message publishing via `Publish::send(provider, topic, message)`. Production Provider structs use empty implementations that delegate to the Omnia SDK defaults:

```rust
impl Publish for Provider {}
```

Usage in domain functions:

```rust
let message = Message::new(serde_json::to_vec(&event)?);
Publish::send(provider, "events-topic", &message).await?;
```

---

## MockProvider Implementation

### Basic Implementation

```rust
use omnia_sdk::{Publish, Message};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct MockProvider {
    published: Arc<Mutex<Vec<Message>>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            published: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn published_messages(&self) -> Vec<Message> {
        self.published.lock().unwrap().clone()
    }
}

impl Publish for MockProvider {
    async fn send(&self, _topic: &str, message: &Message) -> anyhow::Result<()> {
        self.published.lock().unwrap().push(message.clone());
        Ok(())
    }
}
```

### With Topic Verification

```rust
impl Publish for MockProvider {
    async fn send(&self, topic: &str, message: &Message) -> anyhow::Result<()> {
        // Verify topic is expected
        if !topic.starts_with("test-") && topic != "events" {
            anyhow::bail!("unexpected topic: {}", topic);
        }

        self.published.lock().unwrap().push(message.clone());
        Ok(())
    }
}
```

### With Event Deserialization Helper

```rust
impl MockProvider {
    pub fn published_events<T: serde::de::DeserializeOwned>(&self) -> Vec<T> {
        self.published.lock()
            .unwrap()
            .iter()
            .map(|msg| serde_json::from_slice(&msg.payload).unwrap())
            .collect()
    }
}

// Usage in tests:
let events: Vec<YourEvent> = provider.published_events();
assert_eq!(events.len(), 2);
```

### Best Practices

- Capture all published messages
- Use Arc<Mutex<Vec>> for thread-safe capture
- Provide helper to deserialize events
- Verify topic names in tests
- Don't silently drop messages

## References

- [capabilities.md](../capabilities.md) -- Publish trait definition
- [README.md](README.md) -- Provider composition rules
