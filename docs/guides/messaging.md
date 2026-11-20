# Messaging

The `omnia:messaging` interface gives guests topic-based publish/subscribe, request-reply, and an inbound message trigger. The in-memory default (`MessagingDefault`, a broadcast channel) makes all three patterns runnable with zero infrastructure; Kafka (`omnia-kafka`) and NATS (`omnia-nats`) are drop-in production backends. The interface also declares `consumer`, `log`, and `admin` for batched consumption, ordered logs, and topic administration. All three are unstable. The SDK re-exports them raw as `messaging::consumer`, `log`, and `admin`, with no helpers, and every host answers `unsupported` for all three today.

The [`messaging`](../../examples/messaging/) example demonstrates every pattern below with the default backend.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiMessaging: MessagingDefault` (from `omnia_wasi_messaging`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## Publishing

Connect a client by name, build a `Message`, and send it to a topic:

```rust,noplayground
use omnia_sdk::messaging::{self, Message};

let client = messaging::connect(messaging::DEFAULT).await?;
let message = Message::json(&order)?.with_key(&order.customer_id).with_metadata("tenant", "acme");
client.send("orders.created", message).await?;
```

`Message::new(bytes)` builds a message from raw bytes, and `Message::json(&value)` serializes a value under the `application/json` content type. `with_key`, `with_content_type`, and `with_metadata` set the rest. The content type, key, and metadata travel with the message, and on Kafka the key also drives partition assignment. A backend carries every field given or fails with `unsupported`. A portable metadata name matches `[a-z][a-z0-9-]*`. A deployment declares one broker, `default`, and connecting to any other name fails with `not-found`.

## Handling incoming messages

Messaging is a **trigger**: the host (`WasiMessaging`) subscribes to topics and delivers each message to the guest's exported handler, instantiating a fresh guest instance per message:

```rust,noplayground
use omnia_sdk::{Context, Handlers, Router, messaging};

async fn create_order(context: Context, input: CreateOrder) -> Result<(), omnia_sdk::Error> {
    // ...
}

fn router() -> Router {
    Handlers::new().topic("orders.created", create_order).into_router()
}

messaging::export!(router);
```

A guest that writes the raw export implements `omnia_sdk::exports::messaging::incoming_handler::Guest`, binds it with `omnia_sdk::exports::messaging::export!`, and dispatches through `omnia_sdk::messaging::handle` (see [Writing Guests](writing-guests.md#handling-incoming-messages)).

The guest router matches registered topics exactly; broker subscription patterns remain host configuration (`KAFKA_TOPICS`, `NATS_TOPICS`). `topic` decodes JSON and returns `Ok(())` for successful handler output. `Router::handle` reports a failure as an `omnia_sdk::Error`, whose code names it. A payload that does not decode is a `BadRequest` with the code `undecodable`. A delivery with no topic is a `BadRequest` with the code `missing-topic`, and one no route takes is a `NotFound` with the code `unhandled-topic`. A handler's failure takes its class and code from the first `omnia_sdk::Error`, binding error, or typed outcome such as `model::Exhausted` in its source chain, and anything else is a `ServerError`.

What the handler returns settles the delivery. `Ok` accepts it, and a failure crosses as a `Settle` carrying its text as the reason:

- A `BadRequest` rejects the delivery, as a message no redelivery handles. The backend drops or dead-letters it.
- Any other failure releases it, as a failure that may pass. A backend that redelivers delivers the message again.

The router leaves a release's delay to the backend. A guest that sets one writes the raw export and returns the `Settle` itself.

The host sets the delivery's `deadline` to when it stops waiting, and the router hands it to the handler as `Metadata::deadline`. The host logs and counts a settlement, a trap, and a timeout alike. No backend reads the settlement yet: the in-memory default and NATS are finished with a failed message, while Kafka ends its consumer so the process restarts and the record is redelivered (see the [`omnia-kafka` README](https://github.com/augentic/omnia-backends/tree/main/crates/kafka)). In multi-guest deployments, each guest's `routes.messaging` patterns select it by NATS-style topic match — see [Multi-Guest Deployments](multi-guest-deployments.md#routing-inbound-traffic).

For non-JSON payloads, register the route with `topic_with(topic, handler, decode)` instead: the decoder `Fn(&Delivery) -> omnia_sdk::Result<I>` sees the whole delivery (payload, `key`, `content_type`, metadata, `reply_to`, and the broker's `attempt`, `position`, and `published_at` where it reports them) and refuses with an `omnia_sdk::Error`, usually `bad_request!`. Whatever it returns, the router reports a refusal as a `BadRequest` with the code `undecodable` and keeps its description, just as it does malformed JSON under `topic`.

## Request-reply

The requester sends and awaits one reply on the same call; the handler answers the address the delivery carried, through a client of the broker the request came from. A handler that answers registers with `topic_with`, so its decoder can pass the delivery's `reply_to` into the input:

```rust
// Requester
let reply = client.request("quotes", message, None).await?;

// Handler
async fn quote(_: Context, (request, reply_to): (QuoteRequest, Option<String>)) -> omnia_sdk::Result<()> {
    let Some(reply_to) = reply_to else { return Ok(()) };
    let client = messaging::connect(messaging::DEFAULT).await?;
    client.reply(&reply_to, Message::json(&price(&request))?).await
}

Handlers::new().topic_with("quotes", quote, |delivery: &Delivery| {
    Ok((serde_json::from_slice(&delivery.payload)?, delivery.reply_to.clone()))
})
```

The final `request` argument is an optional `Duration`; `None` defers to the backend's bound, and a zero timeout is `invalid`. A delivery's `reply_to` is `None` when the sender awaits no reply.

## Fan-out and async work

Handlers can publish further messages, including many at once. Use `wit_bindgen::spawn_local` for concurrent sends and `yield_async` to keep the instance cooperative:

```rust
for i in 0..1000 {
    wit_bindgen::spawn_local(async move {
        let client = messaging::connect(messaging::DEFAULT).await?;
        client.send("downstream", make_message(i)).await
    });
}
```

A handler can also make outbound HTTP calls mid-message (the example's topic `c` handler does exactly that) — capabilities compose freely inside one guest.

## Combining with HTTP

A single guest can export both the HTTP handler and the messaging handler — a common shape where REST endpoints enqueue work and the messaging handler processes it. The example's HTTP routes (`/pub-sub`, `/request-reply`) each drive one messaging pattern.

## Backends

| Backend | Notes |
| ------- | ----- |
| `MessagingDefault` (in-tree) | In-process broadcast; delivery only within the runtime process |
| `omnia-kafka` | Apache Kafka; `KAFKA_BROKERS`, `COMPONENT`, `KAFKA_TOPICS`, `KAFKA_CONSUMER_GROUP`, SASL via `KAFKA_USERNAME`/`KAFKA_PASSWORD`, optional Schema Registry (`KAFKA_REGISTRY_URL`) |
| `omnia-nats` | NATS core; `NATS_ADDR`, `NATS_TOPICS`, NKey auth via `NATS_JWT`/`NATS_SEED` |

Semantics to keep in mind when moving from the default to a broker: the in-memory backend is at-most-once within one process, while Kafka/NATS bring their own delivery, ordering, and consumer-group semantics. Guest code is unchanged, but handlers should be idempotent if the production broker can redeliver.

## WebSockets

The `omnia:websocket` interface is the same trigger shape pointed at WebSocket connections. The host (`WasiWebSocket`, default backend `WebSocketDefault`, a tungstenite server on `WEBSOCKET_ADDR`) accepts connections, names each with an opaque id, and delivers every message a peer sends to the guest's exported `incoming-handler`. A message is text or binary, and travels in a frame of that type. A guest connects a `Client` with `websocket::connect`, sends to the connections it names with `Client::send`, and reaches every open one with `Client::broadcast`. The handler fails with the wire `types::Error`, so inside it the generated `connections` and `broadcast` calls, which fail with that same type, do the sending:

```rust,noplayground
use omnia_sdk::websocket::types::{Client, Error};
use omnia_sdk::websocket::{self, Event, Message, connections};

// Push text to every connected client
let ws = websocket::connect(websocket::DEFAULT).await?;
ws.broadcast(Message::Text(payload), &[]).await?;

// Receive inbound messages
struct WebSocket;
omnia_sdk::exports::websocket::export!(WebSocket);

impl omnia_sdk::exports::websocket::incoming_handler::Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        // react, or echo back to the sender
        let ws = Client::connect(websocket::DEFAULT.to_owned()).await?;
        connections::send(&ws, vec![event.connection], event.message).await
    }
}
```

The host sets each event's `deadline` to when it stops waiting for `handle`.

The [`websocket`](../../examples/websocket/) example pairs an HTTP control endpoint (POST a message) with a WebSocket broadcast to all connected clients. In manifests, `routes.websocket` entries use the same pattern syntax as messaging routes.
