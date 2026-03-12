# Example 01: Simple HTTP Handler

Complete working example of the Simple Fixture Pattern based on the `ex-http` crate from the context workspace.

> **Note**: Test code in this example uses `std::fs` for loading test fixtures. This is allowed in native `#[cfg(test)]` code only. `std::fs` is **forbidden** in WASM runtime code (`src/`). See [guardrails.md](../references/guardrails.md) for the complete list of WASM constraints.

## Scenario

Generate test harness for a basic HTTP handler crate with two handlers:
- `EchoRequest` - URL decodes and echoes back input
- `GreetingRequest` - Returns greeting with configured name

## Component Structure

```
ex-http/
├── src/
│   ├── lib.rs
│   ├── handlers.rs
│   └── types.rs
├── tests/
│   ├── http.rs          # Test file (to be generated)
│   └── data/
│       ├── echo1.json
│       └── greeting1.json
└── Cargo.toml
```

## Handler Code (Reference)

### handlers.rs

```rust
use omnia_sdk::{Config, Handler};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EchoRequest {
    pub a: String,
    pub b: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EchoResponse {
    pub a: String,
    pub b: String,
}

impl<P: Config> Handler<P> for EchoRequest {
    type Response = EchoResponse;

    async fn handle(self, _provider: &P) -> anyhow::Result<Self::Response> {
        Ok(EchoResponse {
            a: urlencoding::decode(&self.a)?.into_owned(),
            b: self.b,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GreetingRequest {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GreetingResponse {
    pub respondent: String,
    pub reply: String,
}

impl<P: Config> Handler<P> for GreetingRequest {
    type Response = GreetingResponse;

    async fn handle(self, provider: &P) -> anyhow::Result<Self::Response> {
        let name = provider.get("name").await?;
        Ok(GreetingResponse {
            respondent: name,
            reply: self.message,
        })
    }
}
```

## Generated Test Files

### tests/http.rs

```rust
#[cfg(test)]
mod tests {
    use anyhow::bail;
    use ex_http::handlers::{EchoRequest, EchoResponse, GreetingRequest, GreetingResponse};
    use omnia_sdk::{Client, Config};
    use serde::Deserialize;

    struct MockProvider;

    impl Config for MockProvider {
        async fn get(&self, key: &str) -> anyhow::Result<String> {
            match key {
                "name" => Ok("Test User".to_string()),
                _ => bail!(format!("Unknown key: {key}")),
            }
        }
    }

    #[derive(Deserialize)]
    struct GreetingTestCase {
        request: GreetingRequest,
        response: GreetingResponse,
    }

    #[derive(Deserialize)]
    struct EchoTestCase {
        request: EchoRequest,
        response: EchoResponse,
    }

    #[tokio::test]
    async fn echo_responds_with_fixture_values() {
        let fixture = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/echo1.json"));
        let test_case: EchoTestCase =
            serde_json::from_str(fixture).expect("fixture JSON should deserialize");
        let client = Client::new("tester").provider(MockProvider);
        let request = test_case.request.clone();

        let response = client.request(request).await.expect("echo should succeed");

        assert_eq!(response.a, test_case.response.a);
        assert_eq!(response.b, test_case.response.b);
    }

    #[tokio::test]
    async fn greeting_responds_with_fixture_values() {
        let fixture =
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/greeting1.json"));
        let test_case: GreetingTestCase =
            serde_json::from_str(fixture).expect("fixture JSON should deserialize");
        let client = Client::new("tester").provider(MockProvider);
        let request = test_case.request.clone();

        let response = client.request(request).await.expect("greeting should succeed");

        assert_eq!(response.respondent, test_case.response.respondent);
        assert_eq!(response.reply, test_case.response.reply);
    }
}
```

## Test Fixtures

### tests/data/echo1.json

```json
{
    "request": {
        "a": "echo%20this%20back",
        "b": "please"
    },
    "response": {
        "a": "echo this back",
        "b": "please"
    }
}
```

### tests/data/greeting1.json

```json
{
    "request": {
        "message": "Hello, World!"        
    },
    "response": {
        "respondent": "Test User",
        "reply": "Hello, World!"
    }
}
```

## Key Points

### 1. MockProvider Implementation

- **Simple**: Only implements `Config` trait
- **Focused**: Returns only the config values needed by handlers
- **Error handling**: Returns error for unknown keys

### 2. Test Structure

- **Inline fixtures**: Uses `include_str!()` with `concat!()` and `env!("CARGO_MANIFEST_DIR")`
- **Type safety**: Deserializes into typed test case structs
- **Clear assertions**: Compares individual fields

### 3. Client Usage

```rust
let client = Client::new("tester").provider(MockProvider);
let response = client.request(request).await.expect("should succeed");
```

This is the standard pattern - **never call handlers directly**.

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test echo_responds_with_fixture_values

# Run with output
cargo test -- --nocapture
```

### Expected Output

```
running 2 tests
test tests::echo_responds_with_fixture_values ... ok
test tests::greeting_responds_with_fixture_values ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Variations

### Adding More Test Cases

Create additional fixture files:

```json
// tests/data/echo2.json
{
    "request": {
        "a": "hello%20world",
        "b": "test"
    },
    "response": {
        "a": "hello world",
        "b": "test"
    }
}
```

Add corresponding test:

```rust
#[tokio::test]
async fn echo_test_case_2() {
    let fixture = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/echo2.json"));
    let test_case: EchoTestCase = serde_json::from_str(fixture).unwrap();
    let client = Client::new("tester").provider(MockProvider);
    
    let response = client.request(test_case.request).await.unwrap();
    
    assert_eq!(response, test_case.response);
}
```

### Dynamic Fixture Loading

```rust
#[tokio::test]
async fn run_all_echo_fixtures() {
    let data_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data"));
    
    for entry in std::fs::read_dir(data_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap().to_str().unwrap().starts_with("echo") {
            let fixture = std::fs::read_to_string(&path).unwrap();
            let test_case: EchoTestCase = serde_json::from_str(&fixture).unwrap();
            
            let client = Client::new("tester").provider(MockProvider);
            let response = client.request(test_case.request).await.unwrap();
            
            assert_eq!(response, test_case.response, "Failed for {:?}", path);
        }
    }
}
```

## Advantages of This Pattern

1. **Simple** - Minimal boilerplate, easy to understand
2. **Fast** - Quick to implement and run
3. **Type-safe** - Compile-time checking of test data structure
4. **Maintainable** - Clear separation of test logic and data
5. **Standard** - Uses familiar Rust testing patterns

## When to Use

✅ **Use this pattern for**:
- Standard request/response handlers
- Components without time-sensitive logic
- Quick test development
- Most components (90%+ of cases)

❌ **Don't use for**:
- Time-sensitive components (use Replayer pattern)
- Production data replay requirements

## Next Steps

- For StateStore examples, see [testing-statestore.md](testing-statestore.md)
- For Publish examples, see [testing-publisher.md](testing-publisher.md)
- For time-sensitive components, see [replay-writer](../../../../rt/skills/replay-writer/SKILL.md)

## Summary

This example demonstrates the **recommended testing pattern** for most Rust WASM components:
- Simple MockProvider with required traits
- JSON fixtures loaded with `include_str!()`
- Standard tokio async tests
- Client abstraction for handler execution

**Total lines of test code**: ~50 lines
**Setup time**: ~10 minutes
**Maintenance**: Low
