//! Command router public contract.

use std::any::TypeId;
use std::error::Error;
use std::{fmt, io};

use clap::{Args, Command};
use omnia_guest::api::Provider;
use omnia_guest::api::command::{
    self, BuildError, CommandResponse, Completions, Namespace, Outcome, Projector, Router,
    RouterBuilder,
};
use omnia_guest::api::invoke::{CallContext, Invoker};
use omnia_guest::api::operation::Operation;

#[derive(Args, Clone, Debug)]
struct Globals {
    /// Output format.
    #[arg(long, default_value = "text")]
    format: String,
}

#[derive(Args, Debug)]
struct GreetArgs {
    /// Name to greet.
    #[arg(long)]
    name: String,

    /// Return an operation failure.
    #[arg(long)]
    fail: bool,
}

struct GreetInput {
    name: String,
    fail: bool,
}

#[derive(Args)]
struct CollisionArgs {
    #[arg(long = "format")]
    style: String,
}

impl From<CollisionArgs> for GreetInput {
    fn from(args: CollisionArgs) -> Self {
        Self {
            name: args.style,
            fail: false,
        }
    }
}

impl From<GreetArgs> for GreetInput {
    fn from(args: GreetArgs) -> Self {
        Self {
            name: args.name,
            fail: args.fail,
        }
    }
}

#[derive(Debug)]
struct OperationError;

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("operation failed")
    }
}

impl Error for OperationError {}

struct Greet;

impl<P: Provider> Operation<P> for Greet {
    type Error = OperationError;
    type Input = GreetInput;
    type Output = String;

    async fn call(
        input: Self::Input, context: CallContext<'_, P>,
    ) -> Result<Self::Output, Self::Error> {
        if input.fail {
            Err(OperationError)
        } else {
            Ok(format!("hello {}, from {}", input.name, context.owner))
        }
    }
}

#[derive(Clone, Copy)]
struct Text;

impl<D> Projector<String, OperationError, D, Globals> for Text
where
    D: Error + Send + Sync + 'static,
{
    type Error = io::Error;

    fn project(
        &self, outcome: Outcome<String, OperationError, D>, globals: &Globals,
    ) -> Result<CommandResponse, Self::Error> {
        Ok(match outcome {
            Outcome::Output(output) => {
                CommandResponse::success(format!("{}:{output}\n", globals.format))
            }
            Outcome::Operation(error) => CommandResponse::failure(format!("{error}\n"), 7),
            Outcome::Decode(error) => CommandResponse::failure(format!("{error}\n"), 4),
        })
    }

    fn project_failure(&self, error: Self::Error, _globals: &Globals) -> CommandResponse {
        CommandResponse::failure(format!("projection: {error}\n"), 9)
    }
}

fn router() -> Router<(), Globals> {
    RouterBuilder::new(
        Command::new("demo").version("1.2.3").about("Demo commands"),
        Invoker::new("tenant", ()),
    )
    .completions(
        Completions::new()
            .about("Print completion code")
            .long_about("Print completion code from the final grammar."),
    )
    .namespace(
        ["source"],
        Namespace::new().about("Source commands").long_about("Commands for configured sources."),
    )
    .route(
        ["source", "resolve"],
        command::run::<GreetArgs, Greet>()
            .about("Resolve a source")
            .long_about("Resolve one configured source.")
            .alias("get")
            .project_with(Text),
    )
    .route(
        ["source", "internal"],
        command::run::<GreetArgs, Greet>()
            .about("Internal source command")
            .hidden()
            .project_with(Text),
    )
    .build()
    .expect("router should build")
}

#[tokio::test]
async fn dispatch() {
    let router = router();
    let before =
        router.execute(["demo", "--format", "json", "source", "resolve", "--name", "Ada"]).await;
    let after =
        router.execute(["demo", "source", "resolve", "--name", "Ada", "--format", "json"]).await;
    assert_eq!(before, CommandResponse::success("json:hello Ada, from tenant\n"));
    assert_eq!(after, before);

    let alias = router.execute(["demo", "source", "get", "--name", "Ada"]).await;
    assert_eq!(alias.exit, 0);

    let failure = router.execute(["demo", "source", "resolve", "--name", "Ada", "--fail"]).await;
    assert_eq!(failure.exit, 7);
    assert_eq!(failure.stderr, b"operation failed\n");
}

#[tokio::test]
async fn before_dispatch() {
    let router = base()
        .before_dispatch(|globals| {
            (globals.format == "blocked").then(|| CommandResponse::failure("blocked\n", 3))
        })
        .route(["greet"], route())
        .build()
        .expect("router builds");

    let response = router.execute(["demo", "--format", "blocked", "greet", "--name", "Ada"]).await;
    assert_eq!(response, CommandResponse::failure("blocked\n", 3));

    let completions = router.execute(["demo", "--format", "blocked", "completions", "bash"]).await;
    assert_eq!(completions.exit, 0);
}

#[tokio::test]
async fn concurrent_dispatch() {
    let router = router();
    let first = router.execute(["demo", "source", "resolve", "--name", "Ada"]);
    let second = router.execute(["demo", "source", "resolve", "--name", "Grace"]);
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.stdout, b"text:hello Ada, from tenant\n");
    assert_eq!(second.stdout, b"text:hello Grace, from tenant\n");
}

#[tokio::test]
async fn clap_responses() {
    let router = router();

    let help = router.execute(["demo", "source", "resolve", "--help"]).await;
    assert_eq!(help.exit, 0);
    assert!(help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).expect("help is UTF-8");
    assert!(help.contains("Resolve one configured source."));
    assert!(help.contains("--format <FORMAT>"));

    let version = router.execute(["demo", "--version"]).await;
    assert_eq!(version, CommandResponse::success("demo 1.2.3\n"));

    let namespace = router.execute(["demo", "source", "--help"]).await;
    let namespace = String::from_utf8(namespace.stdout).expect("help is UTF-8");
    assert!(namespace.contains("Commands for configured sources."));
    assert!(!namespace.contains("internal"));

    let completion_help = router.execute(["demo", "completions", "--help"]).await;
    let completion_help = String::from_utf8(completion_help.stdout).expect("help is UTF-8");
    assert!(completion_help.contains("Print completion code from the final grammar."));

    let usage = router.execute(["demo", "source", "resolve"]).await;
    assert_eq!(usage.exit, 2);
    assert!(usage.stdout.is_empty());
    assert!(String::from_utf8(usage.stderr).expect("error is UTF-8").contains("Usage:"));
}

#[tokio::test]
async fn completions() {
    let response = router().execute(["demo", "completions", "bash"]).await;
    assert_eq!(response.exit, 0);
    let output = String::from_utf8(response.stdout).expect("completion is UTF-8");
    assert!(output.contains("source"));
    assert!(output.contains("resolve"));
    assert!(output.contains("--format"));
}

#[tokio::test]
async fn argv_zero() {
    let router = router();
    let expected = router.execute(["demo", "source", "resolve"]).await;
    let deployment = router.execute(["demo:guest@1.0.0", "source", "resolve"]).await;
    assert_eq!(deployment, expected);
    assert!(String::from_utf8_lossy(&deployment.stderr).contains("Usage: demo source resolve"));
}

#[test]
fn inventory() {
    let router = router();
    let inventory = router.inventory();
    assert_eq!(inventory.len(), 3);
    let operation = inventory
        .iter()
        .find(|route| route.selector().path() == ["source", "resolve"])
        .expect("operation route");
    assert_eq!(operation.selector().path(), ["source", "resolve"]);
    assert_eq!(operation.operation_type_id(), Some(TypeId::of::<Greet>()));
    assert_eq!(operation.about(), Some("Resolve a source"));
    assert_eq!(operation.aliases(), ["get"]);
    let hidden = inventory
        .iter()
        .find(|route| route.selector().path() == ["source", "internal"])
        .expect("hidden route");
    assert!(hidden.hidden());
    let completion = inventory
        .iter()
        .find(|route| route.operation_type_id().is_none())
        .expect("synthetic route");
    assert_eq!(completion.selector().path(), ["completions"]);
}

#[test]
fn conflicts() {
    let duplicate = base()
        .route(["run"], route())
        .route(["run"], route())
        .build()
        .err()
        .expect("duplicate must fail");
    assert_eq!(duplicate, BuildError::DuplicatePath(vec!["run".to_owned()]));

    let namespace = base()
        .route(["source"], route())
        .route(["source", "resolve"], route())
        .build()
        .err()
        .expect("leaf namespace must fail");
    assert_eq!(namespace, BuildError::LeafNamespace(vec!["source".to_owned()]));

    let alias = base()
        .route(["get"], route())
        .route(["run"], command::run::<GreetArgs, Greet>().alias("get").project_with(Text))
        .build()
        .err()
        .expect("alias collision must fail");
    assert_eq!(alias, BuildError::AliasConflict(Vec::new(), "get".to_owned()));

    let reserved =
        base().route(["completions"], route()).build().err().expect("reserved route must fail");
    assert!(matches!(reserved, BuildError::ReservedCompletion(_)));

    let global = base()
        .route(["run"], command::run::<CollisionArgs, Greet>().project_with(Text))
        .build()
        .err()
        .expect("global collision must fail");
    assert_eq!(global, BuildError::GlobalArgument(vec!["run".to_owned()], "--format".to_owned()));

    let unknown = base()
        .namespace(["missing"], Namespace::new().about("Missing"))
        .route(["run"], route())
        .build()
        .err()
        .expect("unknown namespace must fail");
    assert_eq!(unknown, BuildError::UnknownNamespace(vec!["missing".to_owned()]));

    let leaf = base()
        .namespace(["run"], Namespace::new().about("Run"))
        .route(["run"], route())
        .build()
        .err()
        .expect("leaf namespace metadata must fail");
    assert_eq!(leaf, BuildError::NamespaceLeaf(vec!["run".to_owned()]));

    let duplicate = base()
        .namespace(["source"], Namespace::new().about("Source"))
        .namespace(["source"], Namespace::new().about("Sources"))
        .route(["source", "run"], route())
        .build()
        .err()
        .expect("duplicate namespace metadata must fail");
    assert_eq!(duplicate, BuildError::DuplicateNamespace(vec!["source".to_owned()]));
}

fn base() -> RouterBuilder<(), Globals> {
    RouterBuilder::new(Command::new("demo"), Invoker::new("tenant", ()))
}

fn route() -> command::Binding<GreetArgs, Greet, command::TryIntoDecoder, Text> {
    command::run::<GreetArgs, Greet>().project_with(Text)
}

#[derive(Args)]
struct DecodeArgs {
    #[arg(long)]
    value: u8,
}

struct DecodeInput(u16);
struct Decode;

impl<P: Provider> Operation<P> for Decode {
    type Error = OperationError;
    type Input = DecodeInput;
    type Output = String;

    async fn call(
        input: Self::Input, _context: CallContext<'_, P>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(input.0.to_string())
    }
}

#[tokio::test]
async fn custom_decoder() {
    let router = base()
        .route(
            ["decode"],
            command::run::<DecodeArgs, Decode>()
                .decode_with(|args: DecodeArgs, _globals: &Globals| {
                    if args.value == 0 {
                        Err(OperationError)
                    } else {
                        Ok(DecodeInput(u16::from(args.value) + 1))
                    }
                })
                .project_with(Text),
        )
        .build()
        .expect("custom decoder should build");
    let response = router.execute(["demo", "decode", "--value", "4"]).await;
    assert_eq!(response, CommandResponse::success("text:5\n"));
    let failure = router.execute(["demo", "decode", "--value", "0"]).await;
    assert_eq!(failure.exit, 4);
    assert_eq!(failure.stderr, b"operation failed\n");
}

#[derive(Clone, Copy)]
struct BrokenProjection;

impl<D> Projector<String, OperationError, D, Globals> for BrokenProjection
where
    D: Error + Send + Sync + 'static,
{
    type Error = io::Error;

    fn project(
        &self, _outcome: Outcome<String, OperationError, D>, _globals: &Globals,
    ) -> Result<CommandResponse, Self::Error> {
        Err(io::Error::other("broken renderer"))
    }

    fn project_failure(&self, error: Self::Error, _globals: &Globals) -> CommandResponse {
        CommandResponse::failure(error.to_string(), 23)
    }
}

#[tokio::test]
async fn projection_failure() {
    let router = base()
        .route(["run"], command::run::<GreetArgs, Greet>().project_with(BrokenProjection))
        .build()
        .expect("router should build");
    let response = router.execute(["demo", "run", "--name", "Ada"]).await;
    assert_eq!(response.exit, 23);
    assert_eq!(response.stderr, b"broken renderer");
}

#[test]
fn sinks() {
    struct Broken;

    impl io::Write for Broken {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let response = CommandResponse {
        stdout: b"out".to_vec(),
        stderr: b"err".to_vec(),
        exit: 137,
    };
    assert_eq!(response.exit_code(), std::process::ExitCode::from(137));
    response.write_to(&mut Broken, &mut Vec::new()).unwrap_err();
}
