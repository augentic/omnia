# Messaging

The `omnia:messaging` interface gives guests topic-based publish/subscribe, request-reply, and an inbound message trigger. The in-memory default (`MessagingDefault`, a broadcast channel) makes all three patterns runnable with zero infrastructure; Kafka (`omnia-kafka`) and NATS (`omnia-nats`) are drop-in production backends. The interface also declares `consumer`, `log`, and `admin` for batched consumption, ordered logs, and topic administration, and the host answers `unsupported` for all three today.

The [`messaging`](../../examples/messaging/) example demonstrates every pattern below with the default backend.

> **Host prerequisite.** The runtime serving your guest must link this interface: add `WasiMessaging: MessagingDefault` (from `omnia_wasi_messaging`) to the `runtime!` `hosts:` map — see [Composing a Runtime](composing-a-runtime.md). Everything below is guest code.

## Publishing

Connect a client by name, build a `Message`, and send it to a topic:

```rust
use omnia_wasi_messaging::types::{Client, Message};
use omnia_wasi_messaging::producer;

let client = Client::connect("default".to_string()).await?;
let message = Message {
    data: payload_bytes,
    content_type: Some("application/json".to_string()),
    id: None,
    key: Some("example_key".to_string()),
    metadata: vec![("tenant".to_string(), "acme".to_string())],
};

producer::send(&client, "orders.created".to_string(), message).await?;
```

The content type, key, and metadata travel with the message, and on Kafka the key also drives partition assignment. A backend carries every field given or fails with `unsupported`, and no backend carries an `id` yet. A deployment declares one broker, `default`, and connecting to any other name fails with `not-found`.

A handler reaches the same producer through the `Publisher` capability, which publishes an `omnia_sdk::messaging::Message`. `Message::json` serializes a value under the `application/json` content type, and `with_key` sets its key:

```rust,noplayground
let message = Message::json(&order)?.with_key(&order.customer_id).with_header("tenant", "acme");
context.publisher().publish("orders.created", &message).await?;
```

## Handling incoming messages

Messaging is a **trigger**: the host (`WasiMessaging`) subscribes to topics and delivers each message to the guest's exported handler, instantiating a fresh guest instance per message:

```rust,noplayground
use omnia_sdk::api::{Context, Handlers, Router};
use omnia_wasi_messaging::types::{self, Error};

pub struct Messaging;
omnia_wasi_messaging::export!(Messaging with_types_in omnia_wasi_messaging);

async fn create_order(
    context: Context<MyProvider>, input: CreateOrder,
) -> Result<(), omnia_sdk::Error> {
    // ...
}

fn router() -> Router<MyProvider> {
    Handlers::new().topic("orders.created", create_order).with_state(MyProvider)
}

impl omnia_wasi_messaging::incoming_handler::Guest for Messaging {
    async fn handle(delivery: types::Delivery) -> Result<(), Error> {
        omnia_sdk::wasi::messaging::handle(&router(), delivery).await
    }
}
```

The guest router matches registered topics exactly; broker subscription patterns remain host configuration (`KAFKA_TOPICS`, `NATS_TOPICS`). `topic` decodes JSON and returns `Ok(())` for successful handler output. `Router::handle` reports a failure as an `omnia_sdk::Error`, whose code names it. A payload that does not decode is a `BadRequest` with the code `undecodable`. A delivery with no topic is a `BadRequest` with the code `missing-topic`, and one no route takes is a `NotFound` with the code `unhandled-topic`. A handler's failure takes its class and code from the first `omnia_sdk::Error`, `vcs::Error`, `plugins::Error`, or `model::Error` in its source chain, and anything else is a `ServerError`.

What the handler returns settles the delivery. A failure crosses carrying its text, as the case its class settles as:

- A `BadRequest` is `invalid`, a message no redelivery makes handleable.
- A `BadGateway` is `unavailable`, a failure that may pass.
- Any other failure is `other`, which the backend settles as it decides.

The host logs and counts a returned error, a trap, and a timeout alike. No backend reads the case yet: the in-memory default and NATS are finished with a failed message, while Kafka ends its consumer so the process restarts and the record is redelivered (see the [`omnia-kafka` README](https://github.com/augentic/omnia-backends/tree/main/crates/kafka)). In multi-guest deployments, each guest's `routes.messaging` patterns select it by NATS-style topic match — see [Multi-Guest Deployments](multi-guest-deployments.md#routing-inbound-traffic).

For non-JSON payloads, register the route with `topic_with(topic, handler, decode)` instead: the decoder `Fn(&Delivery) -> omnia_sdk::Result<I>` sees the whole delivery (payload, `key`, `content_type`, metadata) and refuses with an `omnia_sdk::Error`, usually `bad_request!`. Whatever it returns, the router reports a refusal as a `BadRequest` with the code `undecodable` and keeps its description, just as it does malformed JSON under `topic`.

## Request-reply

The requester sends and awaits one reply on the same call; the handler answers the address the delivery carried:

```rust
// Requester
let reply = request_reply::request(&client, "quotes".to_string(), message, None).await?;

// Handler (inside incoming_handler::Guest::handle)
if let Some(reply_to) = delivery.reply_to {
    request_reply::reply(reply_to, response).await?;
}
```

The final `request` argument is an optional timeout in milliseconds; `None` defers to the backend's bound, and zero is `invalid`. A delivery's `reply_to` is `None` when the sender awaits no reply.

## Fan-out and async work

Handlers can publish further messages, including many at once. Use `wit_bindgen::spawn_local` for concurrent sends and `yield_async` to keep the instance cooperative:

```rust
for i in 0..1000 {
    wit_bindgen::spawn_local(async move {
        let client = Client::connect("default".to_string()).await?;
        producer::send(&client, "downstream".to_string(), make_message(i)).await
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

The `omnia:websocket` interface is the same trigger shape pointed at WebSocket connections. The host (`WasiWebSocket`, default backend `WebSocketDefault`, a tungstenite server on `WEBSOCKET_ADDR`) accepts connections, names each with an opaque id, and delivers every message a peer sends to the guest's exported `incoming-handler`. A message is text or binary, and travels in a frame of that type. Guests answer one connection through `connections`, and reach every open one through `broadcast`:

```rust
use omnia_wasi_websocket::types::{Client, Error, Event, Message};
use omnia_wasi_websocket::{broadcast, connections};

// Push text to every connected client
let ws = Client::connect("default".to_string()).await?;
broadcast::send(&ws, Message::Text(payload), Vec::new()).await?;

// Receive inbound messages
struct WebSocket;
omnia_wasi_websocket::export!(WebSocket);

impl omnia_wasi_websocket::incoming_handler::Guest for WebSocket {
    async fn handle(event: Event) -> Result<(), Error> {
        // react, or echo back to the sender
        let ws = Client::connect("default".to_string()).await?;
        connections::send(&ws, event.connection, event.message).await
    }
}
```

A handler sends through the `WebSockets` capability instead: `broadcast(data)` reaches every open connection, and `send_to(sockets, data)` the connections named, each in a binary frame.

The [`websocket`](../../examples/websocket/) example pairs an HTTP control endpoint (POST a message) with a WebSocket broadcast to all connected clients. In manifests, `routes.websocket` entries use the same pattern syntax as messaging routes.
