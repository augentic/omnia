//! Smoke-test every example against its README contract: start each host
//! binary, drive the documented requests/argv, and assert status codes and
//! exit codes. Assumes guests and hosts are already built — run via
//! `cargo make smoke`, which builds both first.
//!
//! Expected skips: `identity` fails fast without IDENTITY_* credentials, and
//! the http-proxy origin routes need outbound internet.

use std::fs::{self, File};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

type HttpClient = Client<HttpConnector, Full<Bytes>>;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const HELLO: &str = r#"{"text":"hello"}"#;

enum Scenario {
    /// Spawn `<bin> run [wasm]`, wait for port 8080, drive HTTP checks, stop.
    Server { name: &'static str, wasm: Option<&'static str>, checks: &'static [Check] },
    /// Run `<bin> [run <wasm> --] <args>` to completion and assert its exit.
    Command {
        name: &'static str,
        bin: &'static str,
        wasm: Option<&'static str>,
        args: &'static [&'static str],
        expect: Expect,
    },
    /// A flow with bespoke semantics.
    Custom(fn(&Ctx) -> Vec<Outcome>),
}

impl Scenario {
    const fn server(
        name: &'static str, wasm: Option<&'static str>, checks: &'static [Check],
    ) -> Self {
        Self::Server { name, wasm, checks }
    }

    const fn cmd(
        name: &'static str, bin: &'static str, wasm: Option<&'static str>,
        args: &'static [&'static str], expect: Expect,
    ) -> Self {
        Self::Command {
            name,
            bin,
            wasm,
            args,
            expect,
        }
    }
}

struct Check {
    label: &'static str,
    // Held as a string so `Check` has no destructor, letting the check
    // arrays const-promote through the `Scenario::server` call.
    method: &'static str,
    path: &'static str,
    json_body: Option<&'static str>,
    expect_status: u16,
    expect_body_contains: Option<&'static str>,
    requires_internet: bool,
}

impl Check {
    const fn req(
        method: &'static str, label: &'static str, path: &'static str,
        json_body: Option<&'static str>,
    ) -> Self {
        Self {
            label,
            method,
            path,
            json_body,
            expect_status: 200,
            expect_body_contains: None,
            requires_internet: false,
        }
    }

    const fn get(label: &'static str, path: &'static str) -> Self {
        Self::req("GET", label, path, None)
    }

    const fn post(label: &'static str, path: &'static str, body: &'static str) -> Self {
        Self::req("POST", label, path, Some(body))
    }

    const fn status(mut self, status: u16) -> Self {
        self.expect_status = status;
        self
    }

    const fn body_contains(mut self, needle: &'static str) -> Self {
        self.expect_body_contains = Some(needle);
        self
    }

    const fn needs_internet(mut self) -> Self {
        self.requires_internet = true;
        self
    }
}

enum Expect {
    /// Exit 0.
    Ok,
    /// Exit 0 with a case-insensitive needle in the output log.
    OkWith(&'static str),
    ExitCode(i32),
}

enum Outcome {
    Pass(String),
    Fail(String),
    Skip(String),
    Warn(String),
}

impl Outcome {
    fn line(&self) -> String {
        match self {
            Self::Pass(msg) => format!("PASS {msg}"),
            Self::Fail(msg) => format!("FAIL {msg}"),
            Self::Skip(msg) => format!("SKIP {msg}"),
            Self::Warn(msg) => format!("WARN {msg}"),
        }
    }
}

const SCENARIOS: &[Scenario] = &[
    Scenario::server(
        "http",
        Some("http_wasm.wasm"),
        &[Check::post("post", "/", HELLO), Check::get("get", "/")],
    ),
    Scenario::server("keyvalue", Some("keyvalue_wasm.wasm"), &[Check::post("post", "/", HELLO)]),
    Scenario::server("blobstore", Some("blobstore_wasm.wasm"), &[Check::post("post", "/", HELLO)]),
    Scenario::server("vault", Some("vault_wasm.wasm"), &[Check::post("post", "/", HELLO)]),
    Scenario::server(
        "sql",
        Some("sql_wasm.wasm"),
        &[
            Check::post(
                "create-agency",
                "/agencies",
                r#"{"agency_id":1,"name":"Ritchies Transport","url":"https://ritchies.co.nz","timezone":"Pacific/Auckland"}"#,
            ),
            Check::get("list-agencies", "/agencies"),
            Check::req(
                "PATCH",
                "patch-agency",
                "/agencies/1",
                Some(r#"{"name":"Ritchies Transport Agency","timezone":"Pacific/Auckland"}"#),
            ),
            Check::post(
                "create-feed",
                "/agencies/1/feeds",
                r#"{"feed_id":1,"description":"Bus routes and schedules"}"#,
            ),
            Check::get("list-feeds", "/feeds"),
            Check::req("DELETE", "delete-feed", "/feeds/1", None),
        ],
    ),
    Scenario::server(
        "docstore",
        Some("docstore_wasm.wasm"),
        &[
            Check::post(
                "create1",
                "/stops",
                r#"{"id":"stop-001","stop_name":"Britomart Transport Centre","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}"#,
            ),
            Check::post(
                "create2",
                "/stops",
                r#"{"id":"stop-002","stop_name":"Newmarket Station","stop_lat":-36.8690,"stop_lon":174.7779,"zone_id":"zone-1"}"#,
            ),
            Check::post(
                "create3",
                "/stops",
                r#"{"id":"stop-003","stop_name":"Albany Station","stop_lat":-36.7275,"stop_lon":174.6986,"zone_id":"zone-3"}"#,
            ),
            Check::get("get", "/stops/stop-001"),
            Check::req(
                "PUT",
                "put",
                "/stops/stop-001",
                Some(
                    r#"{"stop_name":"Britomart","stop_lat":-36.8442,"stop_lon":174.7676,"zone_id":"zone-1"}"#,
                ),
            ),
            Check::get("query-all", "/stops"),
            Check::get("query-text", "/stops?q=Station"),
            Check::get("query-zone", "/stops?zone=zone-1"),
            Check::get("query-lat", "/stops?min_lat=-36.90&max_lat=-36.80"),
            Check::get("query-limit", "/stops?limit=2"),
            Check::req("DELETE", "delete", "/stops/stop-003", None),
        ],
    ),
    Scenario::server("otel", Some("otel_wasm.wasm"), &[Check::post("post", "/", HELLO)]),
    Scenario::server(
        "http-proxy",
        Some("http_proxy_wasm.wasm"),
        &[
            Check::get("cache1", "/cache"),
            Check::get("cache2", "/cache"),
            // Proxies jsonplaceholder.cypress.io; skipped without internet.
            Check::get("origin-sm", "/origin-sm").needs_internet(),
        ],
    ),
    Scenario::server(
        "messaging",
        Some("messaging_wasm.wasm"),
        &[Check::post("pub-sub", "/pub-sub", HELLO)],
    ),
    Scenario::server("config", Some("config_wasm.wasm"), &[Check::get("get", "/")]),
    Scenario::server("websocket", Some("websocket_wasm.wasm"), &[Check::post("post", "/", HELLO)]),
    Scenario::server(
        "mcp",
        Some("mcp_wasm.wasm"),
        &[
            Check::post(
                "tools-list",
                "/mcp/docs",
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ),
            Check::post(
                "tools-call",
                "/mcp/docs",
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_doc","arguments":{"name":"overview"}}}"#,
            ),
        ],
    ),
    // Manifest compiled in, so `run` takes no wasm argument.
    Scenario::server(
        "http-routing",
        None,
        &[
            Check::get("route-a", "/a").body_contains("guest a"),
            Check::get("route-b", "/b"),
            Check::get("route-c-404", "/c").status(404),
        ],
    ),
    Scenario::cmd("model/run", "model", None, &[], Expect::Ok),
    Scenario::cmd(
        "cli/greet",
        "cli",
        Some("cli_wasm.wasm"),
        &["greet", "Ada"],
        Expect::OkWith("ada"),
    ),
    Scenario::cmd(
        "cli/fail-42",
        "cli",
        Some("cli_wasm.wasm"),
        &["fail", "42"],
        Expect::ExitCode(42),
    ),
    Scenario::cmd("cli/bogus", "cli", Some("cli_wasm.wasm"), &["bogus"], Expect::ExitCode(2)),
    Scenario::cmd("cli/fail", "cli", Some("cli_wasm.wasm"), &["fail"], Expect::ExitCode(1)),
    // cli-static has no `run` grammar; argv is the command itself.
    Scenario::cmd("cli-static/greet", "cli-static", None, &["greet", "Ada"], Expect::OkWith("ada")),
    Scenario::cmd("cli-static/add", "cli-static", None, &["add", "2", "40"], Expect::Ok),
    Scenario::cmd("cli-static/fail-42", "cli-static", None, &["fail", "42"], Expect::ExitCode(42)),
    Scenario::Custom(run_guest_link),
    Scenario::cmd("guest-link-dynamic", "guest-link-dynamic", None, &[], Expect::Ok),
    Scenario::cmd("guest-link-register", "guest-link-register", None, &[], Expect::Ok),
    Scenario::Custom(run_identity),
];

struct Ctx {
    bin: PathBuf,
    wasm: PathBuf,
    log: PathBuf,
    rust_log: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().context("workspace root")?;
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let ctx = Ctx {
        bin: target.join("debug/examples"),
        wasm: target.join("wasm32-wasip2/debug/examples"),
        log: std::env::temp_dir().join(format!("omnia-smoke-{}", std::process::id())),
        rust_log: std::env::var("RUST_LOG").unwrap_or_else(|_| "info,opentelemetry_sdk=off".into()),
    };
    fs::create_dir_all(&ctx.log)?;

    if port_open() {
        bail!("port 8080 is already in use; stop that server first");
    }

    let client: HttpClient = Client::builder(TokioExecutor::new()).build_http();
    let mut results = Vec::new();
    for scenario in SCENARIOS {
        let outcomes = match scenario {
            Scenario::Server { name, wasm, checks } => {
                run_server(&ctx, &client, name, *wasm, checks).await
            }
            Scenario::Command {
                name,
                bin,
                wasm,
                args,
                expect,
            } => run_command(&ctx, name, bin, *wasm, args, expect),
            Scenario::Custom(run) => run(&ctx),
        };
        for outcome in outcomes {
            println!("{}", outcome.line());
            results.push(outcome);
        }
    }

    let pass = results.iter().filter(|o| matches!(o, Outcome::Pass(_))).count();
    let fail = results.iter().filter(|o| matches!(o, Outcome::Fail(_))).count();
    println!();
    println!("===== SUMMARY =====");
    println!("pass: {pass}");
    println!("fail: {fail}");
    for outcome in &results {
        if !matches!(outcome, Outcome::Pass(_)) {
            println!("{}", outcome.line());
        }
    }
    println!("logs: {}", ctx.log.display());
    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn spawn_logged(ctx: &Ctx, bin: &str, log_path: &Path, args: &[&str]) -> Child {
    let log_file = File::create(log_path).expect("create log file");
    Command::new(ctx.bin.join(bin))
        .args(args)
        .env("RUST_LOG", &ctx.rust_log)
        .stdout(Stdio::from(log_file.try_clone().expect("clone log file")))
        .stderr(Stdio::from(log_file))
        .spawn()
        .unwrap_or_else(|err| panic!("spawning {bin}: {err}"))
}

async fn run_server(
    ctx: &Ctx, client: &HttpClient, name: &str, wasm: Option<&str>, checks: &[Check],
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let mut args = vec!["run".to_string()];
    if let Some(wasm) = wasm {
        args.push(ctx.wasm.join(wasm).display().to_string());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = spawn_logged(ctx, name, &ctx.log.join(format!("{name}.log")), &args);

    let mut started = false;
    for _ in 0..120 {
        if port_open() {
            started = true;
            break;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            outcomes.push(Outcome::Fail(format!("{name}/startup (process died)")));
            stop_server(&mut child, &mut outcomes);
            return outcomes;
        }
        sleep(Duration::from_millis(500));
    }
    if !started {
        outcomes.push(Outcome::Fail(format!("{name}/startup (port 8080 never opened)")));
        stop_server(&mut child, &mut outcomes);
        return outcomes;
    }

    for check in checks {
        if check.requires_internet && !internet_available() {
            outcomes.push(Outcome::Skip(format!("{name}/{} (no outbound internet)", check.label)));
            continue;
        }
        run_check(client, name, check, &mut outcomes).await;
    }
    stop_server(&mut child, &mut outcomes);
    outcomes
}

async fn run_check(client: &HttpClient, name: &str, check: &Check, outcomes: &mut Vec<Outcome>) {
    let label = check.label;
    match request(client, check).await {
        Ok((status, body)) => {
            if status == check.expect_status {
                outcomes.push(Outcome::Pass(format!("{name}/{label} ({status})")));
            } else {
                outcomes.push(Outcome::Fail(format!(
                    "{name}/{label} (got {status} want {}) body={}",
                    check.expect_status,
                    snippet(&body)
                )));
            }
            if let Some(needle) = check.expect_body_contains {
                if body.contains(needle) {
                    outcomes.push(Outcome::Pass(format!("{name}/{label}-body")));
                } else {
                    outcomes.push(Outcome::Fail(format!("{name}/{label}-body body={body}")));
                }
            }
        }
        Err(err) => {
            outcomes.push(Outcome::Fail(format!("{name}/{label} (request failed: {err:#})")))
        }
    }
}

async fn request(client: &HttpClient, check: &Check) -> Result<(u16, String)> {
    let uri: hyper::Uri = format!("http://localhost:8080{}", check.path).parse()?;
    let mut builder = hyper::Request::builder().method(check.method).uri(uri);
    if check.json_body.is_some() {
        builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
    }
    let body = Full::new(Bytes::from_static(check.json_body.unwrap_or("").as_bytes()));
    let response = tokio::time::timeout(REQUEST_TIMEOUT, client.request(builder.body(body)?))
        .await
        .context("request timed out")??;
    let status = response.status().as_u16();
    let body = tokio::time::timeout(REQUEST_TIMEOUT, response.into_body().collect())
        .await
        .context("body read timed out")??
        .to_bytes();
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

fn run_command(
    ctx: &Ctx, name: &str, bin: &str, wasm: Option<&str>, args: &[&str], expect: &Expect,
) -> Vec<Outcome> {
    let log_path = ctx.log.join(format!("{}.log", name.replace('/', "-")));
    let wasm_path = wasm.map(|wasm| ctx.wasm.join(wasm).display().to_string());
    let mut argv = Vec::new();
    if let Some(wasm_path) = &wasm_path {
        argv.extend(["run", wasm_path, "--"]);
    }
    argv.extend(args);
    let mut child = spawn_logged(ctx, bin, &log_path, &argv);
    let status = child.wait().expect("wait for child");
    let code = status.code().unwrap_or(-1);
    let outcome = match expect {
        Expect::Ok => {
            if code == 0 {
                Outcome::Pass(format!("{name} (exit 0)"))
            } else {
                Outcome::Fail(format!("{name} (exit {code})"))
            }
        }
        Expect::OkWith(needle) => {
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            if code == 0 && log.to_lowercase().contains(needle) {
                Outcome::Pass(name.to_string())
            } else {
                Outcome::Fail(format!("{name} (exit {code}) {}", snippet(&log)))
            }
        }
        Expect::ExitCode(want) => {
            if code == *want {
                Outcome::Pass(format!("{name} (exit {want})"))
            } else {
                Outcome::Fail(format!("{name} (exit {code}, want {want})"))
            }
        }
    };
    vec![outcome]
}

/// The host either runs the link demo to completion or stays up; both are
/// healthy as long as the log is clean.
fn run_guest_link(ctx: &Ctx) -> Vec<Outcome> {
    let log_path = ctx.log.join("guest-link.log");
    let mut child = spawn_logged(ctx, "guest-link", &log_path, &["run"]);
    sleep(Duration::from_secs(8));
    let outcome = match child.try_wait().expect("poll guest-link") {
        None => {
            let log = fs::read_to_string(&log_path).unwrap_or_default().to_lowercase();
            let outcome = if log.contains("error") || log.contains("panic") {
                Outcome::Fail("guest-link/run (errors in log)".into())
            } else {
                Outcome::Pass("guest-link/run (host up, clean log)".into())
            };
            let _ = child.kill();
            let _ = child.wait();
            outcome
        }
        Some(status) if status.success() => Outcome::Pass("guest-link/run (exited 0)".into()),
        Some(status) => {
            Outcome::Fail(format!("guest-link/run (exit {})", status.code().unwrap_or(-1)))
        }
    };
    vec![outcome]
}

/// Expected to fail fast on missing IDENTITY_* credentials. The backend
/// connection is checked ~10s into startup, so allow up to 60s.
fn run_identity(ctx: &Ctx) -> Vec<Outcome> {
    let log_path = ctx.log.join("identity.log");
    let wasm = ctx.wasm.join("identity_wasm.wasm").display().to_string();
    let mut child = spawn_logged(ctx, "identity", &log_path, &["run", &wasm]);
    let mut exited = None;
    for _ in 0..60 {
        if let Some(status) = child.try_wait().expect("poll identity") {
            exited = Some(status);
            break;
        }
        sleep(Duration::from_secs(1));
    }
    let outcome = match exited {
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Outcome::Fail(
                "identity/run (still running without credentials; expected fail-fast)".into(),
            )
        }
        Some(status) => {
            let code = status.code().unwrap_or(-1);
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            if log.to_uppercase().contains("IDENTITY_") {
                Outcome::Skip(format!(
                    "identity (fail-fast on missing IDENTITY_* vars, exit {code} — expected)"
                ))
            } else {
                Outcome::Fail(format!(
                    "identity (exit {code} without the expected missing-vars message)"
                ))
            }
        }
    };
    vec![outcome]
}

fn stop_server(child: &mut Child, outcomes: &mut Vec<Outcome>) {
    let _ = child.kill();
    let _ = child.wait();
    for _ in 0..40 {
        if !port_open() {
            return;
        }
        sleep(Duration::from_millis(500));
    }
    outcomes.push(Outcome::Warn("port 8080 still open after stop".into()));
}

fn port_open() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

/// Probe the origin the http-proxy example fronts; a TCP connect to its
/// HTTPS port is enough to distinguish "offline" from "origin reachable".
fn internet_available() -> bool {
    let Ok(mut addrs) = "jsonplaceholder.cypress.io:443".to_socket_addrs() else {
        return false;
    };
    addrs
        .next()
        .is_some_and(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(10)).is_ok())
}

/// First 200 characters, matching the old script's `head -c 200` diagnostics.
fn snippet(text: &str) -> String {
    text.chars().take(200).collect()
}
