# Broadcast Capability Example

**Demonstrates:** `Broadcast` capability trait for WebSocket replies

## Overview

The `Broadcast` trait enables sending data to connected WebSocket clients. It is the Omnia equivalent of `ws.send()`, `socket.write()`, or `connection.send()` in source code. Use it whenever a handler needs to send data back over a WebSocket channel -- whether broadcasting to all clients or replying to a specific connection.

**Trait definition:** See [../../references/capabilities.md](../../references/capabilities.md#broadcast)

## Simple WebSocket Reply

A handler that receives a WebSocket event and sends a reply back to all clients on the channel:

```rust
use anyhow::Context as _;
use omnia_sdk::{
    Broadcast, Config, Context, Error, Handler, Reply, Result,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PingEvent {
    pub client_id: String,
}

#[derive(Clone, Debug, Serialize)]
struct PongResponse {
    status: String,
    client_id: String,
}

const WS_CHANNEL: &str = "default";

async fn handle_ping<P>(
    _owner: &str,
    provider: &P,
    event: PingEvent,
) -> Result<()>
where
    P: Config + Broadcast,
{
    let response = PongResponse {
        status: "ok".to_string(),
        client_id: event.client_id,
    };
    let payload = serde_json::to_vec(&response)
        .context("serializing PongResponse")?;

    Broadcast::send(provider, WS_CHANNEL, &payload, None).await?;

    Ok(())
}

impl<P: Config + Broadcast> Handler<P> for PingEvent {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = ();

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing PingEvent")
            .map_err(Into::into)
    }

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<()>> {
        handle_ping(ctx.owner, ctx.provider, self).await?;
        Ok(Reply::ok(()))
    }
}
```

## WebSocket Protocol Handshake

A handler that implements a multi-step authentication handshake over WebSocket. Each incoming event triggers a specific reply:

```rust
use anyhow::Context as _;
use omnia_sdk::{Broadcast, Config, Result};
use serde::{Deserialize, Serialize};

const WS_CHANNEL: &str = "default";

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ProtocolEvent {
    #[serde(rename = "auth_status")]
    AuthStatus { status: String },
    #[serde(rename = "server_list")]
    ServerList { servers: Vec<String> },
    #[serde(rename = "data")]
    Data { payload: serde_json::Value },
}

#[derive(Clone, Debug, Serialize)]
struct ListRequest {
    r#type: String,
}

#[derive(Clone, Debug, Serialize)]
struct SelectRequest {
    r#type: String,
    server_id: String,
}

async fn handle_auth_status<P>(provider: &P, status: &str) -> Result<()>
where
    P: Config + Broadcast,
{
    if status != "success" {
        return Err(anyhow::anyhow!("authentication failed: {status}").into());
    }

    let request = ListRequest {
        r#type: "list_request".to_string(),
    };
    let payload = serde_json::to_vec(&request)
        .context("serializing ListRequest")?;
    Broadcast::send(provider, WS_CHANNEL, &payload, None).await?;
    Ok(())
}

async fn handle_server_list<P>(provider: &P, servers: &[String]) -> Result<()>
where
    P: Config + Broadcast,
{
    let server_id = servers.first()
        .ok_or_else(|| anyhow::anyhow!("empty server list"))?;

    let request = SelectRequest {
        r#type: "select_server".to_string(),
        server_id: server_id.clone(),
    };
    let payload = serde_json::to_vec(&request)
        .context("serializing SelectRequest")?;
    Broadcast::send(provider, WS_CHANNEL, &payload, None).await?;
    Ok(())
}
```

## Targeted WebSocket Send

Sending to specific WebSocket clients rather than broadcasting to all:

```rust
async fn send_to_client<P: Broadcast>(
    provider: &P,
    channel: &str,
    socket_id: &str,
    event: &impl Serialize,
) -> Result<()> {
    let payload = serde_json::to_vec(event)
        .context("serializing targeted event")?;
    let targets = vec![socket_id.to_string()];
    Broadcast::send(provider, channel, &payload, Some(targets)).await?;
    Ok(())
}
```

## Combined WebSocket + Publish

A handler that receives WebSocket events, sends a reply back, and also publishes to a message topic:

```rust
use omnia_sdk::{Broadcast, Config, Message, Publish, Result};

const WS_CHANNEL: &str = "default";
const OUTPUT_TOPIC: &str = "position-updates.v1";

async fn handle_position<P>(provider: &P, position: &PositionUpdate) -> Result<()>
where
    P: Config + Broadcast + Publish,
{
    // Acknowledge receipt to the WebSocket client
    let ack = serde_json::to_vec(&Ack { status: "received" })?;
    Broadcast::send(provider, WS_CHANNEL, &ack, None).await?;

    // Publish to Kafka topic for downstream consumers
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{OUTPUT_TOPIC}");
    let payload = serde_json::to_vec(position)?;
    Publish::send(provider, &topic, &Message::new(&payload)).await?;

    Ok(())
}
```

## Key Patterns

1. **Channel name as constant** -- Define the WebSocket channel name as a `const &str` (e.g., `"default"`)
2. **`None` for broadcast, `Some(vec![...])` for targeted** -- Third argument controls recipient scope
3. **Serialize to `Vec<u8>`** -- `Broadcast::send` takes `&[u8]`, use `serde_json::to_vec`
4. **Combine with `Publish`** -- WebSocket handlers often need both `Broadcast` (reply) and `Publish` (message queue)
5. **Not just for "broadcasting"** -- Despite the name, `Broadcast` is also used for point-to-point WebSocket replies

## References

- See [../../references/capabilities.md](../../references/capabilities.md#broadcast) for the full `Broadcast` trait definition
- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/guest-patterns.md](../../references/guest-patterns.md#websocket-handler-setup) for WebSocket guest wiring
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
