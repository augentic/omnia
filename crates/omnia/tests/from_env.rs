//! The `FromEnv` derive, driven through the loaders it emits: `load_from`
//! over a scripted lookup, and `load_env` and `with_env` over the variables
//! cargo sets in every test process.

#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::time::Duration;

use omnia::FromEnv;

#[derive(Debug, PartialEq, Eq, FromEnv)]
#[env(crate = omnia)]
struct Options {
    #[env(from = "PORT", default = "8080")]
    port: u16,
    #[env(from = "HOST")]
    host: String,
    #[env(from = "TOKEN")]
    token: Option<String>,
    #[env(from = "TOPICS", default = "orders", with = omnia::env::list)]
    topics: Vec<String>,
    #[env(from = "TIMEOUT_MS", default = "250", with = omnia::env::millis)]
    timeout: Duration,
}

#[derive(Debug, PartialEq, Eq, FromEnv)]
#[env(crate = omnia)]
struct Retry {
    #[env(from = "RETRY_ATTEMPTS", default = "3")]
    attempts: u32,
    #[env(from = "RETRY_BACKOFF_MS", default = "100", with = omnia::env::millis)]
    backoff: Duration,
}

// `vars` answers; every other variable is unset
fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| vars.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
}

#[test]
fn unset_variables() {
    let retry = Retry::load_from(|_| None).expect("every field has a default");
    assert_eq!(
        retry,
        Retry {
            attempts: 3,
            backoff: Duration::from_millis(100)
        }
    );
    assert_eq!(retry, Retry::default());

    let err = Options::load_from(|_| None).expect_err("`HOST` has no default");
    assert_eq!(err.to_string(), "`HOST` is required but unset");
}

#[test]
fn set_variables() {
    let vars = [
        ("PORT", "9090"),
        ("HOST", "example.test"),
        ("TOKEN", "t0ken"),
        ("TOPICS", "orders,, invoices "),
        ("TIMEOUT_MS", "1500"),
    ];
    let options = Options::load_from(lookup(&vars)).expect("every value parses");
    assert_eq!(
        options,
        Options {
            port: 9090,
            host: "example.test".to_owned(),
            token: Some("t0ken".to_owned()),
            topics: vec!["orders".to_owned(), "invoices".to_owned()],
            timeout: Duration::from_millis(1500),
        }
    );
}

#[test]
fn bad_variables() {
    let vars = [("PORT", "eighty"), ("TIMEOUT_MS", "-5")];
    let err = Options::load_from(lookup(&vars)).expect_err("two values do not parse");

    // one error, in field order, naming each variable but never its value
    assert_eq!(
        err.to_string(),
        "`PORT` is invalid: invalid digit found in string; `HOST` is required but unset; \
         `TIMEOUT_MS` is invalid: invalid digit found in string"
    );
}

#[derive(Debug, FromEnv)]
#[env(crate = omnia)]
struct Package {
    #[env(from = "CARGO_PKG_NAME")]
    name: String,
    #[env(from = "OMNIA_FROM_ENV_UNSET")]
    unset: String,
}

// cargo sets `CARGO_PKG_NAME` in every test process, so the process
// environment is read without being written
#[test]
fn with_env() {
    assert!(std::env::var_os("OMNIA_FROM_ENV_UNSET").is_none());

    let package = Package {
        name: "in code".to_owned(),
        unset: "in code".to_owned(),
    }
    .with_env()
    .expect("every field has a value");
    assert_eq!(package.name, env!("CARGO_PKG_NAME"));
    assert_eq!(package.unset, "in code");

    let err = <Package as omnia::FromEnv>::load_env().expect_err("`OMNIA_FROM_ENV_UNSET` is unset");
    assert_eq!(err.to_string(), "`OMNIA_FROM_ENV_UNSET` is required but unset");
}

#[derive(Debug, FromEnv)]
#[env(crate = omnia, validate = Self::check)]
struct Window {
    #[env(from = "WINDOW_MIN", default = "0")]
    min: u32,
    #[env(from = "WINDOW_MAX", default = "10")]
    max: u32,
}

impl Window {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.min <= self.max,
            "WINDOW_MIN ({}) exceeds WINDOW_MAX ({})",
            self.min,
            self.max
        );
        Ok(())
    }
}

#[test]
fn validate() {
    let err = Window::load_from(lookup(&[("WINDOW_MIN", "20")])).expect_err("min exceeds max");
    assert_eq!(err.to_string(), "WINDOW_MIN (20) exceeds WINDOW_MAX (10)");

    let err = Window { min: 20, max: 10 }.with_env().expect_err("min exceeds max");
    assert_eq!(err.to_string(), "WINDOW_MIN (20) exceeds WINDOW_MAX (10)");

    // a value that does not parse fails the load before `validate` runs
    let err = Window::load_from(lookup(&[("WINDOW_MIN", "x")])).expect_err("min does not parse");
    assert_eq!(err.to_string(), "`WINDOW_MIN` is invalid: invalid digit found in string");
}

#[derive(Debug, PartialEq, Eq, FromEnv)]
#[env(crate = omnia)]
struct Proxy {
    #[env(from = "PROXY_URL")]
    url: String,
    #[env(from = "PROXY_PORT", default = "3128")]
    port: u16,
}

#[derive(Debug, PartialEq, Eq, FromEnv)]
#[env(crate = omnia)]
struct Client {
    #[env(nested)]
    proxy: Option<Proxy>,
    #[env(nested)]
    retry: Retry,
}

#[test]
fn nested() {
    let client = Client::load_from(|_| None).expect("`proxy` is optional and `retry` has defaults");
    assert_eq!(
        client,
        Client {
            proxy: None,
            retry: Retry::default()
        }
    );

    let vars = [("PROXY_URL", "http://proxy.test"), ("RETRY_ATTEMPTS", "5")];
    let client = Client::load_from(lookup(&vars)).expect("every value parses");
    assert_eq!(
        client.proxy,
        Some(Proxy {
            url: "http://proxy.test".to_owned(),
            port: 3128
        })
    );
    assert_eq!(client.retry.attempts, 5);

    // a proxy value that does not parse is reported, never read as no proxy
    let vars = [("PROXY_PORT", "x"), ("RETRY_ATTEMPTS", "y")];
    let err = Client::load_from(lookup(&vars)).expect_err("two values do not parse");
    assert_eq!(
        err.to_string(),
        "`PROXY_URL` is required but unset; `PROXY_PORT` is invalid: invalid digit found in \
         string; `RETRY_ATTEMPTS` is invalid: invalid digit found in string"
    );
}

#[derive(Debug, FromEnv)]
#[env(crate = omnia)]
struct Backend {
    #[env(from = "OMNIA_FROM_ENV_MODEL", default = "auto")]
    model: String,
    #[env(default = "https://backend.test")]
    url: String,
}

#[test]
fn code_only() {
    let asked = RefCell::new(Vec::new());
    let backend = Backend::load_from(|name| {
        asked.borrow_mut().push(name.to_owned());
        Some("from the environment".to_owned())
    })
    .expect("every field has a value");
    assert_eq!(asked.into_inner(), ["OMNIA_FROM_ENV_MODEL"]);
    assert_eq!(backend.model, "from the environment");
    assert_eq!(backend.url, "https://backend.test");

    let backend = Backend {
        url: "https://other.test".to_owned(),
        ..Backend::default()
    }
    .with_env()
    .expect("every field has a value");
    assert_eq!(backend.url, "https://other.test");
}

#[derive(Debug, FromEnv)]
#[env(crate = omnia)]
struct Late {
    #[env(from = "LATE_TIMEOUT_MS", default = "soon", with = omnia::env::millis)]
    timeout: Duration,
}

#[test]
#[should_panic(expected = "the default `soon` of `Late.timeout` does not parse")]
fn bad_default() {
    let _ = Late::load_from(|_| None);
}

#[test]
fn compile_fail_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/from_env_invalid.rs");
}
