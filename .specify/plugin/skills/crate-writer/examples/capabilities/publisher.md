# Publish Capability Example

**Demonstrates:** `Publish` and `Message` capability traits

## Overview

The `Publish` trait enables publishing messages to a variety of messaging services. Messages are constructed using the `Message` struct, which carries a byte payload and optional headers.

**Trait definition:** See [../../references/capabilities.md](../../references/capabilities.md#publisher)

## Simple Message Publishing

A handler that processes an incoming request and publishes an event:

```rust
use anyhow::Context as _;
use omnia_sdk::{
    Config, Context, Error, Handler, Message, Publish, Reply, Result,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrderRequest {
    pub order_id: String,
    pub customer_id: String,
    pub total: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrderResponse {
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
struct OrderCreatedEvent {
    order_id: String,
    customer_id: String,
    total: f64,
}

const ORDER_TOPIC: &str = "order-created.v1";

async fn create_order<P>(
    _owner: &str,
    provider: &P,
    req: OrderRequest,
) -> Result<OrderResponse>
where
    P: Config + Publish,
{
    // Build event payload
    let event = OrderCreatedEvent {
        order_id: req.order_id.clone(),
        customer_id: req.customer_id,
        total: req.total,
    };
    let payload = serde_json::to_vec(&event)
        .context("serializing OrderCreatedEvent")?;

    // Construct message
    let message = Message::new(&payload);

    // Build topic with env prefix and publish
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{ORDER_TOPIC}");
    Publish::send(provider, &topic, &message).await?;

    Ok(OrderResponse {
        status: format!("Order {} created", req.order_id),
    })
}

impl<P: Config + Publish> Handler<P> for OrderRequest {
    type Error = Error;
    type Input = Vec<u8>;
    type Output = OrderResponse;

    async fn handle(self, ctx: Context<'_, P>) -> Result<Reply<OrderResponse>> {
        Ok(create_order(ctx.owner, ctx.provider, self).await?.into())
    }

    fn from_input(input: Self::Input) -> Result<Self> {
        serde_json::from_slice(&input)
            .context("deserializing OrderRequest")
            .map_err(Into::into)
    }
}
```

## Multi-Topic Publishing

Publishing to multiple topics in a single handler:

```rust
const OUTPUT_TOPIC: &str = "events-output.v1";
const AUDIT_TOPIC: &str = "events-audit.v1";

async fn process_event<P>(provider: &P, event: &IncomingEvent) -> Result<()>
where
    P: Config + Publish,
{
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());

    let payload = serde_json::to_vec(event)?;
    let message = Message::new(&payload);

    // Publish to primary output topic
    let output_topic = format!("{env}-{OUTPUT_TOPIC}");
    Publish::send(provider, &output_topic, &message).await?;

    // Publish to audit topic
    let audit_topic = format!("{env}-{AUDIT_TOPIC}");
    let audit_event = AuditEntry {
        event_type: "event_processed".to_string(),
        event_id: event.id.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let audit_payload = serde_json::to_vec(&audit_event)?;
    let audit_message = Message::new(&audit_payload);
    Publish::send(provider, &audit_topic, &audit_message).await?;

    Ok(())
}
```

## Messages with Headers

Adding metadata headers to messages:

```rust
async fn publish_with_headers<P: Config + Publish>(
    provider: &P,
    event: &Event,
    correlation_id: &str,
) -> Result<()> {
    let payload = serde_json::to_vec(event)?;
    let mut message = Message::new(&payload);

    // Add headers for downstream consumers
    message.headers.insert(
        "correlation-id".to_string(),
        correlation_id.to_string(),
    );
    message.headers.insert(
        "content-type".to_string(),
        "application/json".to_string(),
    );
    message.headers.insert(
        "source".to_string(),
        "order-service".to_string(),
    );

    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let topic = format!("{env}-{OUTPUT_TOPIC}");
    Publish::send(provider, &topic, &message).await?;

    Ok(())
}
```

## Authenticated Publishing

Combining `Publish` with `Identity` and `HttpRequest` for a complete event-driven flow:

```rust
async fn enrich_and_publish<P>(provider: &P, raw_event: &RawEvent) -> Result<()>
where
    P: Config + Identity + HttpRequest + Publish,
{
    // 1. Authenticate to fetch enrichment data
    let identity = Config::get(provider, "AZURE_IDENTITY").await?;
    let token = Identity::access_token(provider, identity).await?;

    // 2. Enrich event via authenticated API call
    let api_url = Config::get(provider, "ENRICHMENT_API_URL").await?;
    let request = http::Request::builder()
        .method("POST")
        .uri(&api_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(raw_event)?.into())?;

    let response = HttpRequest::fetch(provider, request).await?;
    let enriched: EnrichedEvent = serde_json::from_slice(response.body())?;

    // 3. Publish enriched event
    let env = Config::get(provider, "ENV").await.unwrap_or_else(|_| "dev".to_string());
    let payload = serde_json::to_vec(&enriched)?;
    let message = Message::new(&payload);
    let topic = format!("{env}-{OUTPUT_TOPIC}");
    Publish::send(provider, &topic, &message).await?;

    Ok(())
}
```

## Key Patterns

1. **Hardcoded base topics with env prefix** -- Define base topic names as `const &str`, read `ENV` from Config, format as `{env}-{TOPIC}`. See [capabilities.md](../../references/capabilities.md#topic-naming-pattern).
2. **Serialize to `Vec<u8>`** -- `Message::new` takes `&[u8]`, use `serde_json::to_vec`
3. **One message per send** -- call `Publish::send` once per message; there is no batch API
4. **Headers are optional** -- use `message.headers` only when downstream consumers need metadata
5. **Config keys in `.env.example`** -- document the `ENV` key

```bash
# .env.example
ENV=dev
```

## References

- See [../../references/capabilities.md](../../references/capabilities.md) for the full `Publish` and `Message` definitions
- See [identity.md](identity.md) for the authentication flow
- See [../../references/sdk-api.md](../../references/sdk-api.md) for the Handler trait pattern
- See [../../references/providers.md](../../references/providers.md) for provider bound composition
