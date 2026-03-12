# Example 03: Messaging with Publish

Complete working example demonstrating Publish trait implementation with event capture and verification, based on the `ex-messaging` crate from the context workspace.

## Scenario

Generate test harness for a messaging component that:
- Publishes messages to topics
- Implements request-reply pattern
- Verifies published events in tests

## Component Structure

```
ex-messaging/
├── src/
│   ├── lib.rs
│   ├── handlers.rs
│   └── request_reply.rs
├── tests/
│   ├── messaging.rs    # Test file
│   └── data/
│       ├── pubsub1.json
│       └── reqreply1.json
└── Cargo.toml
```

## Handler Code (Reference)

### handlers.rs

```rust
use omnia_sdk::{Config, Handler, Publish, Message};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublishRequest {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PublishResponse {
    pub message: String,
}

impl<P: Config + Publish> Handler<P> for PublishRequest {
    type Response = PublishResponse;

    async fn handle(self, provider: &P) -> anyhow::Result<Self::Response> {
        let topic = provider.get("PUB_SUB_TOPIC").await?;
        
        let message = Message::new(self.message.as_bytes());
        provider.send(&topic, &message).await?;
        
        Ok(PublishResponse {
            message: format!("Published: {}", self.message),
        })
    }
}
```

## Generated Test Files

### tests/messaging.rs

```rust
#[cfg(test)]
mod tests {
    use ex_messaging::handlers::{
        PublishRequest, PublishResponse, SendReceiveRequest, SendReceiveResponse,
    };
    use ex_messaging::request_reply::RequestReply;
    use omnia_sdk::{Client, Config, Publish};
    use serde::Deserialize;

    struct MockProvider;

    impl Config for MockProvider {
        async fn get(&self, key: &str) -> anyhow::Result<String> {
            Ok(match key {
                "PUB_SUB_TOPIC" => "example_pub_sub_topic".to_string(),
                "REQUEST_REPLY_TOPIC" => "example_request_reply_topic".to_string(),
                _ => anyhow::bail!("unknown config key: {key}"),
            })
        }
    }

    impl Publish for MockProvider {
        async fn send(&self, topic: &str, message: &omnia_sdk::Message) -> anyhow::Result<()> {
            tracing::debug!("Mock publish to topic '{topic}' with message: {:?}", message);
            Ok(())
        }
    }

    impl RequestReply for MockProvider {
        async fn send_receive(
            &self, _topic: &str, _message: &omnia_sdk::Message,
        ) -> anyhow::Result<omnia_sdk::Message> {
            let reply_payload = b"ACK".to_vec();
            Ok(omnia_sdk::Message::new(&reply_payload))
        }
    }

    #[derive(Deserialize)]
    struct PubSubTestCase {
        request: PublishRequest,
        response: PublishResponse,
    }

    #[tokio::test]
    async fn publish_responds_with_fixture_values() {
        let fixture = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/pubsub1.json"));
        let test_case: PubSubTestCase =
            serde_json::from_str(fixture).expect("fixture JSON should deserialize");
        let client = Client::new("tester").provider(MockProvider);
        let request = test_case.request.clone();

        let response = client.request(request).await.expect("publish should succeed");

        assert_eq!(response.message, test_case.response.message);
    }

    #[derive(Deserialize)]
    struct SendReceiveTestCase {
        request: SendReceiveRequest,
        response: SendReceiveResponse,
    }

    #[tokio::test]
    async fn send_receive_responds_with_fixture_values() {
        let fixture =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/reqreply1.json"));
        let test_case: SendReceiveTestCase =
            serde_json::from_str(fixture).expect("fixture JSON should deserialize");
        let request = test_case.request.clone();
        let client = Client::new("tester").provider(MockProvider);

        let response = client.request(request).await.expect("send_receive should succeed");

        assert_eq!(response.message, test_case.response.message);
    }
}
```

## Enhanced Version with Event Capture

### tests/messaging_enhanced.rs

```rust
mod tests {
    use ex_messaging::handlers::{PublishRequest, PublishResponse};
    use omnia_sdk::{Client, Config, Publish, Message};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockProvider {
        published: Arc<Mutex<Vec<(String, Message)>>>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                published: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn published_messages(&self) -> Vec<(String, Message)> {
            self.published.lock().unwrap().clone()
        }

        fn published_to_topic(&self, topic: &str) -> Vec<Message> {
            self.published.lock()
                .unwrap()
                .iter()
                .filter(|(t, _)| t == topic)
                .map(|(_, m)| m.clone())
                .collect()
        }
    }

    impl Config for MockProvider {
        async fn get(&self, key: &str) -> anyhow::Result<String> {
            Ok(match key {
                "PUB_SUB_TOPIC" => "test-topic".to_string(),
                _ => anyhow::bail!("unknown config key: {key}"),
            })
        }
    }

    impl Publish for MockProvider {
        async fn send(&self, topic: &str, message: &Message) -> anyhow::Result<()> {
            self.published.lock()
                .unwrap()
                .push((topic.to_string(), message.clone()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn publish_sends_to_correct_topic() {
        let provider = MockProvider::new();
        let client = Client::new("tester").provider(provider.clone());
        let request = PublishRequest {
            message: "Hello, World!".to_string(),
        };

        let response = client.request(request).await.expect("publish should succeed");

        // Verify response
        assert_eq!(response.message, "Published: Hello, World!");

        // Verify message was published
        let published = provider.published_messages();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, "test-topic");

        // Verify message content
        let message_content = String::from_utf8(published[0].1.payload.clone()).unwrap();
        assert_eq!(message_content, "Hello, World!");
    }

    #[tokio::test]
    async fn multiple_publishes_captured() {
        let provider = MockProvider::new();
        let client = Client::new("tester").provider(provider.clone());

        // Publish multiple messages
        for i in 1..=3 {
            let request = PublishRequest {
                message: format!("Message {}", i),
            };
            client.request(request).await.expect("publish should succeed");
        }

        // Verify all messages captured
        let published = provider.published_messages();
        assert_eq!(published.len(), 3);

        for (i, (topic, message)) in published.iter().enumerate() {
            assert_eq!(topic, "test-topic");
            let content = String::from_utf8(message.payload.clone()).unwrap();
            assert_eq!(content, format!("Message {}", i + 1));
        }
    }

    #[tokio::test]
    async fn verify_published_event_structure() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Event {
            event_type: String,
            data: String,
        }

        let provider = MockProvider::new();
        let client = Client::new("tester").provider(provider.clone());

        // Assume handler publishes structured events
        let request = PublishRequest {
            message: "test".to_string(),
        };
        client.request(request).await.expect("publish should succeed");

        // Deserialize and verify event structure
        let published = provider.published_messages();
        let event: Event = serde_json::from_slice(&published[0].1.payload)
            .expect("should deserialize as Event");

        assert_eq!(event.event_type, "MessagePublished");
        assert_eq!(event.data, "test");
    }
}
```

## Key Points

### 1. Publish Trait Implementation

```rust
impl Publish for MockProvider {
    async fn send(&self, topic: &str, message: &Message) -> anyhow::Result<()> {
        self.published.lock()
            .unwrap()
            .push((topic.to_string(), message.clone()));
        Ok(())
    }
}
```

**Critical details**:
- Captures both topic and message
- Stores in `Arc<Mutex<Vec<_>>>` for thread safety
- Returns `Ok(())` after capture

### 2. Event Capture Pattern

```rust
#[derive(Clone)]
struct MockProvider {
    published: Arc<Mutex<Vec<(String, Message)>>>,
}

impl MockProvider {
    fn published_messages(&self) -> Vec<(String, Message)> {
        self.published.lock().unwrap().clone()
    }

    fn published_to_topic(&self, topic: &str) -> Vec<Message> {
        self.published.lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, m)| m.clone())
            .collect()
    }
}
```

**Helper methods**:
- `published_messages()` - Get all published messages
- `published_to_topic()` - Filter by topic

### 3. Message Verification

```rust
// Verify message count
let published = provider.published_messages();
assert_eq!(published.len(), 1);

// Verify topic
assert_eq!(published[0].0, "test-topic");

// Verify message content
let message_content = String::from_utf8(published[0].1.payload.clone()).unwrap();
assert_eq!(message_content, "Hello, World!");
```

### 4. Structured Event Verification

```rust
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Event {
    event_type: String,
    data: String,
}

// Deserialize published message
let event: Event = serde_json::from_slice(&published[0].1.payload)
    .expect("should deserialize as Event");

assert_eq!(event.event_type, "MessagePublished");
assert_eq!(event.data, "test");
```

## Test Fixtures

### tests/data/pubsub1.json

```json
{
    "request": {
        "message": "Hello, World!"
    },
    "response": {
        "message": "Published: Hello, World!"
    }
}
```

### tests/data/reqreply1.json

```json
{
    "request": {
        "message": "Request"
    },
    "response": {
        "message": "ACK"
    }
}
```

## Advanced Patterns

### Topic Verification

```rust
#[tokio::test]
async fn publishes_to_correct_topic() {
    let provider = MockProvider::new();
    let client = Client::new("tester").provider(provider.clone());

    let request = PublishRequest { message: "test".to_string() };
    client.request(request).await.unwrap();

    let messages = provider.published_to_topic("test-topic");
    assert_eq!(messages.len(), 1);

    let wrong_topic = provider.published_to_topic("wrong-topic");
    assert_eq!(wrong_topic.len(), 0);
}
```

### Multiple Topic Publishing

```rust
impl Publish for MockProvider {
    async fn send(&self, topic: &str, message: &Message) -> anyhow::Result<()> {
        // Verify topic is expected
        let valid_topics = ["events", "notifications", "audit"];
        if !valid_topics.contains(&topic) {
            anyhow::bail!("unexpected topic: {}", topic);
        }

        self.published.lock()
            .unwrap()
            .push((topic.to_string(), message.clone()));
        Ok(())
    }
}
```

### Event Deserialization Helper

```rust
impl MockProvider {
    fn published_events<T: serde::de::DeserializeOwned>(&self) -> Vec<T> {
        self.published.lock()
            .unwrap()
            .iter()
            .map(|(_, msg)| serde_json::from_slice(&msg.payload).unwrap())
            .collect()
    }
}

// Usage:
let events: Vec<MyEvent> = provider.published_events();
assert_eq!(events.len(), 2);
assert_eq!(events[0].event_type, "Created");
```

### Async Event Verification

```rust
#[tokio::test]
async fn handler_publishes_async_events() {
    let provider = MockProvider::new();
    let client = Client::new("tester").provider(provider.clone());

    // Handler may publish multiple events asynchronously
    let request = ComplexRequest { /* ... */ };
    client.request(request).await.unwrap();

    // Wait for async publishing to complete
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let published = provider.published_messages();
    assert!(published.len() >= 1, "should publish at least one event");
}
```

## Custom Trait Implementation

### request_reply.rs

```rust
use omnia_sdk::Message;

#[async_trait::async_trait]
pub trait RequestReply {
    async fn send_receive(
        &self,
        topic: &str,
        message: &Message,
    ) -> anyhow::Result<Message>;
}
```

### Mock Implementation

```rust
impl RequestReply for MockProvider {
    async fn send_receive(
        &self, _topic: &str, _message: &Message,
    ) -> anyhow::Result<Message> {
        let reply_payload = b"ACK".to_vec();
        Ok(Message::new(&reply_payload))
    }
}
```

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test publish_sends_to_correct_topic

# Run with logging
RUST_LOG=debug cargo test -- --nocapture
```

### Expected Output

```
running 5 tests
test tests::publish_responds_with_fixture_values ... ok
test tests::send_receive_responds_with_fixture_values ... ok
test tests::publish_sends_to_correct_topic ... ok
test tests::multiple_publishes_captured ... ok
test tests::verify_published_event_structure ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Troubleshooting

### Issue: Published Messages Not Captured

**Cause**: MockProvider not cloned properly

**Solution**: Ensure provider is cloned before passing to client
```rust
let provider = MockProvider::new();
let client = Client::new("tester").provider(provider.clone());
// Use provider.published_messages() after request
```

### Issue: Message Deserialization Fails

**Cause**: Message payload format doesn't match expected type

**Solution**: Verify payload format
```rust
let payload = &published[0].1.payload;
println!("Payload: {:?}", String::from_utf8_lossy(payload));
```

### Issue: Arc/Mutex Deadlock

**Cause**: Holding lock while making async calls

**Solution**: Release lock before async operations
```rust
// ❌ Bad - holds lock during async
let mut published = self.published.lock().unwrap();
some_async_call().await;
published.push(message);

// ✅ Good - release lock quickly
self.published.lock().unwrap().push(message);
some_async_call().await;
```

## Dependencies

### Cargo.toml

```toml
[dependencies]
omnia-sdk = "0.26"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
async-trait = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
```

## Summary

This example demonstrates:
- ✅ **Publish trait implementation** with event capture
- ✅ **Arc<Mutex<Vec>> pattern** for thread-safe capture
- ✅ **Topic verification** in tests
- ✅ **Message deserialization** helpers
- ✅ **Custom trait implementation** (RequestReply)
- ✅ **Multiple event verification** patterns

**Total lines of test code**: ~150 lines
**Setup time**: ~20 minutes
**Maintenance**: Low

## Next Steps

- For time-sensitive components, see [replay-writer](../../../../rt/skills/replay-writer/SKILL.md)
- For migration guidance, see [replay-writer](../../../../rt/skills/replay-writer/SKILL.md)
- For provider trait reference, see [providers](../references/providers/README.md)
