use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use clap::error::ErrorKind;
use clap::{Arg, ArgMatches, Args, Command, FromArgMatches};
use clap_complete::Shell;

use super::builder::{Binding, Decoder, Outcome, Projector};
use super::response::CommandResponse;
use crate::api::Provider;
use crate::api::invocation::Invocation;
use crate::api::invoke::Invoker;
use crate::api::operation::Operation;

const COMPLETIONS: &str = "completions";

/// Top-level command metadata.
#[derive(Clone, Debug)]
pub struct App {
    name: &'static str,
    version: Option<&'static str>,
    about: Option<&'static str>,
    long_about: Option<&'static str>,
    completions: Completions,
}

impl App {
    /// Create application metadata.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            version: None,
            about: None,
            long_about: None,
            completions: Completions::new(),
        }
    }

    /// Set the application version.
    #[must_use]
    pub const fn version(mut self, version: &'static str) -> Self {
        self.version = Some(version);
        self
    }

    /// Set short application help.
    #[must_use]
    pub const fn about(mut self, about: &'static str) -> Self {
        self.about = Some(about);
        self
    }

    /// Set detailed application help.
    #[must_use]
    pub const fn long_about(mut self, long_about: &'static str) -> Self {
        self.long_about = Some(long_about);
        self
    }

    /// Configure the synthetic completions command.
    #[must_use]
    pub const fn completions(mut self, completions: Completions) -> Self {
        self.completions = completions;
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.name);
        if let Some(version) = self.version {
            command = command.version(version);
        }
        if let Some(about) = self.about {
            command = command.about(about);
        }
        if let Some(long_about) = self.long_about {
            command = command.long_about(long_about);
        }
        command
    }
}

/// Help metadata for an intermediate command namespace.
#[derive(Clone, Copy, Debug, Default)]
pub struct Namespace {
    about: Option<&'static str>,
    long_about: Option<&'static str>,
}

impl Namespace {
    /// Create empty namespace metadata.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            about: None,
            long_about: None,
        }
    }

    /// Set short namespace help.
    #[must_use]
    pub const fn about(mut self, about: &'static str) -> Self {
        self.about = Some(about);
        self
    }

    /// Set detailed namespace help.
    #[must_use]
    pub const fn long_about(mut self, long_about: &'static str) -> Self {
        self.long_about = Some(long_about);
        self
    }
}

/// Help metadata for the synthetic completions command.
#[derive(Clone, Copy, Debug)]
pub struct Completions {
    about: &'static str,
    long_about: Option<&'static str>,
}

impl Completions {
    /// Create the default completions metadata.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            about: "Generate shell completions",
            long_about: None,
        }
    }

    /// Set short completions help.
    #[must_use]
    pub const fn about(mut self, about: &'static str) -> Self {
        self.about = about;
        self
    }

    /// Set detailed completions help.
    #[must_use]
    pub const fn long_about(mut self, long_about: &'static str) -> Self {
        self.long_about = Some(long_about);
        self
    }
}

impl Default for Completions {
    fn default() -> Self {
        Self::new()
    }
}

/// No application-wide command arguments.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoGlobals;

impl Args for NoGlobals {
    fn augment_args(cmd: Command) -> Command {
        cmd
    }

    fn augment_args_for_update(cmd: Command) -> Command {
        cmd
    }
}

impl FromArgMatches for NoGlobals {
    fn from_arg_matches(_matches: &ArgMatches) -> Result<Self, clap::Error> {
        Ok(Self)
    }

    fn from_arg_matches_mut(_matches: &mut ArgMatches) -> Result<Self, clap::Error> {
        Ok(Self)
    }

    fn update_from_arg_matches(&mut self, _matches: &ArgMatches) -> Result<(), clap::Error> {
        Ok(())
    }

    fn update_from_arg_matches_mut(
        &mut self, _matches: &mut ArgMatches,
    ) -> Result<(), clap::Error> {
        Ok(())
    }
}

/// A command route selector.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Selector {
    path: Vec<String>,
}

impl Selector {
    /// Return the nested command path.
    #[must_use]
    pub fn path(&self) -> &[String] {
        &self.path
    }
}

/// Read-only metadata for one command binding.
#[derive(Clone, Debug)]
pub struct RouteInfo {
    selector: Selector,
    operation_type_id: Option<TypeId>,
    about: Option<&'static str>,
    long_about: Option<&'static str>,
    aliases: Vec<&'static str>,
    hidden: bool,
}

impl RouteInfo {
    /// Return the command selector.
    #[must_use]
    pub const fn selector(&self) -> &Selector {
        &self.selector
    }

    /// Return the bound operation type, if this is not a synthetic route.
    #[must_use]
    pub const fn operation_type_id(&self) -> Option<TypeId> {
        self.operation_type_id
    }

    /// Return short route help.
    #[must_use]
    pub const fn about(&self) -> Option<&'static str> {
        self.about
    }

    /// Return detailed route help.
    #[must_use]
    pub const fn long_about(&self) -> Option<&'static str> {
        self.long_about
    }

    /// Return route aliases.
    #[must_use]
    pub fn aliases(&self) -> &[&'static str] {
        &self.aliases
    }

    /// Return whether the route is hidden from parent help.
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.hidden
    }
}

/// A command grammar construction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildError {
    /// A route has no command segments.
    EmptyPath,
    /// A route contains an empty command segment.
    EmptySegment,
    /// More than one route uses the same path.
    DuplicatePath(Vec<String>),
    /// One path is both an executable leaf and a namespace.
    LeafNamespace(Vec<String>),
    /// A command name or alias is ambiguous within a namespace.
    AliasConflict(Vec<String>, String),
    /// A route claims the synthetic completions namespace.
    ReservedCompletion(Vec<String>),
    /// A global argument collides with a leaf argument.
    GlobalArgument(Vec<String>, String),
    /// More than one metadata entry targets the same namespace.
    DuplicateNamespace(Vec<String>),
    /// Namespace metadata targets a path absent from the route trie.
    UnknownNamespace(Vec<String>),
    /// Namespace metadata targets an executable leaf.
    NamespaceLeaf(Vec<String>),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => f.write_str("command route path cannot be empty"),
            Self::EmptySegment => f.write_str("command route segments cannot be empty"),
            Self::DuplicatePath(path) => {
                write!(f, "duplicate command route `{}`", join(path))
            }
            Self::LeafNamespace(path) => {
                write!(f, "command route `{}` is both a leaf and namespace", join(path))
            }
            Self::AliasConflict(path, name) => {
                write!(f, "command name or alias `{name}` conflicts under `{}`", join(path))
            }
            Self::ReservedCompletion(path) => {
                write!(f, "command route `{}` uses reserved completions name", join(path))
            }
            Self::GlobalArgument(path, argument) => write!(
                f,
                "leaf argument `{argument}` on `{}` conflicts with a global argument",
                join(path)
            ),
            Self::DuplicateNamespace(path) => {
                write!(f, "duplicate command namespace metadata for `{}`", join(path))
            }
            Self::UnknownNamespace(path) => {
                write!(f, "command namespace `{}` has no registered routes", join(path))
            }
            Self::NamespaceLeaf(path) => {
                write!(f, "command namespace `{}` is an executable leaf", join(path))
            }
        }
    }
}

impl std::error::Error for BuildError {}

fn join(path: &[String]) -> String {
    if path.is_empty() { "<root>".to_owned() } else { path.join(" ") }
}

type DispatchFuture<'a> = Pin<Box<dyn Future<Output = CommandResponse> + Send + 'a>>;
type BeforeDispatch<G> = Arc<dyn Fn(&G) -> Option<CommandResponse> + Send + Sync>;

trait ErasedRoute<P: Provider, G>: Send + Sync {
    fn operation_type_id(&self) -> TypeId;
    fn about(&self) -> Option<&'static str>;
    fn long_about(&self) -> Option<&'static str>;
    fn aliases(&self) -> &[&'static str];
    fn hidden(&self) -> bool;
    fn argument_keys(&self) -> Vec<String>;
    fn command(&self, name: &'static str) -> Command;
    fn dispatch<'a>(
        &'a self, matches: &'a ArgMatches, globals: &'a G, invoker: &'a Invoker<P>,
    ) -> DispatchFuture<'a>;
}

struct Route<P, G, A, O, D, Q> {
    binding: Binding<A, O, D, Q>,
    marker: PhantomData<fn(P, G)>,
}

impl<P, G, A, O, D, Q> ErasedRoute<P, G> for Route<P, G, A, O, D, Q>
where
    P: Provider,
    G: Send + Sync + 'static,
    A: Args + FromArgMatches + Send + 'static,
    O: Operation<P>,
    O::Input: Send + 'static,
    O::Output: Send + 'static,
    O::Error: Send + Sync + 'static,
    D: Decoder<A, O::Input, G>,
    Q: Projector<O::Output, O::Error, D::Error, G>,
{
    fn operation_type_id(&self) -> TypeId {
        TypeId::of::<O>()
    }

    fn about(&self) -> Option<&'static str> {
        self.binding.about
    }

    fn long_about(&self) -> Option<&'static str> {
        self.binding.long_about
    }

    fn aliases(&self) -> &[&'static str] {
        &self.binding.aliases
    }

    fn hidden(&self) -> bool {
        self.binding.hidden
    }

    fn argument_keys(&self) -> Vec<String> {
        argument_keys(&A::augment_args(Command::new("leaf"))).into_iter().collect()
    }

    fn command(&self, name: &'static str) -> Command {
        let mut command = A::augment_args(Command::new(name));
        if let Some(about) = self.binding.about {
            command = command.about(about);
        }
        if let Some(long_about) = self.binding.long_about {
            command = command.long_about(long_about);
        }
        for alias in &self.binding.aliases {
            command = command.visible_alias(alias);
        }
        command = command.hide(self.binding.hidden);
        command
    }

    fn dispatch<'a>(
        &'a self, matches: &'a ArgMatches, globals: &'a G, invoker: &'a Invoker<P>,
    ) -> DispatchFuture<'a> {
        Box::pin(async move {
            let args = match A::from_arg_matches(matches) {
                Ok(args) => args,
                Err(error) => return clap_error(&error),
            };
            let input = match self.binding.decoder.decode(args, globals) {
                Ok(input) => input,
                Err(error) => {
                    return project(&self.binding.projector, Outcome::Decode(error), globals);
                }
            };
            let outcome = match invoker.invoke::<O>(Invocation::new(input)).await {
                Ok(output) => Outcome::Output(output),
                Err(error) => Outcome::Operation(error),
            };
            project(&self.binding.projector, outcome, globals)
        })
    }
}

fn project<T, O, D, G, Q>(projector: &Q, outcome: Outcome<T, O, D>, globals: &G) -> CommandResponse
where
    Q: Projector<T, O, D, G>,
{
    match projector.project(outcome, globals) {
        Ok(response) => response,
        Err(error) => projector.project_failure(error, globals),
    }
}

struct Registration<P: Provider, G> {
    path: Vec<&'static str>,
    route: Arc<dyn ErasedRoute<P, G>>,
}

struct NamespaceRegistration {
    path: Vec<&'static str>,
    metadata: Namespace,
}

struct Node<P: Provider, G> {
    leaf: Option<Arc<dyn ErasedRoute<P, G>>>,
    children: BTreeMap<&'static str, Self>,
    namespace: Option<Namespace>,
}

impl<P: Provider, G> Default for Node<P, G> {
    fn default() -> Self {
        Self {
            leaf: None,
            children: BTreeMap::new(),
            namespace: None,
        }
    }
}

/// A typed command router with application-wide arguments.
pub struct Router<P: Provider, G = NoGlobals> {
    app: App,
    invoker: Invoker<P>,
    before_dispatch: Option<BeforeDispatch<G>>,
    registrations: Vec<Registration<P, G>>,
    namespaces: Vec<NamespaceRegistration>,
    command: Option<Command>,
    routes: BTreeMap<Vec<String>, Arc<dyn ErasedRoute<P, G>>>,
    inventory: Vec<RouteInfo>,
}

impl<P: Provider> Router<P, NoGlobals> {
    /// Create an empty command router.
    #[must_use]
    pub fn new(app: App, invoker: Invoker<P>) -> Self {
        Self {
            app,
            invoker,
            before_dispatch: None,
            registrations: Vec::new(),
            namespaces: Vec::new(),
            command: None,
            routes: BTreeMap::new(),
            inventory: Vec::new(),
        }
    }

    /// Set the application-wide clap argument type.
    #[must_use]
    pub fn globals<G>(self) -> Router<P, G>
    where
        G: Args + FromArgMatches + Send + Sync + 'static,
    {
        Router {
            app: self.app,
            invoker: self.invoker,
            before_dispatch: None,
            registrations: Vec::new(),
            namespaces: Vec::new(),
            command: None,
            routes: BTreeMap::new(),
            inventory: Vec::new(),
        }
    }
}

impl<P, G> Router<P, G>
where
    P: Provider,
    G: Args + FromArgMatches + Send + Sync + 'static,
{
    /// Run application policy after globals parse and before route dispatch.
    ///
    /// Returning a response stops dispatch and preserves its output channels
    /// and exit status.
    #[must_use]
    pub fn before_dispatch(
        mut self, hook: impl Fn(&G) -> Option<CommandResponse> + Send + Sync + 'static,
    ) -> Self {
        self.before_dispatch = Some(Arc::new(hook));
        self
    }

    /// Attach help metadata to an intermediate command namespace.
    #[must_use]
    pub fn namespace<I>(mut self, path: I, metadata: Namespace) -> Self
    where
        I: IntoIterator<Item = &'static str>,
    {
        self.command = None;
        self.routes.clear();
        self.inventory.clear();
        self.namespaces.push(NamespaceRegistration {
            path: path.into_iter().collect(),
            metadata,
        });
        self
    }

    /// Register one typed operation route.
    #[must_use]
    pub fn route<I, A, O, D, Q>(mut self, path: I, binding: Binding<A, O, D, Q>) -> Self
    where
        I: IntoIterator<Item = &'static str>,
        A: Args + FromArgMatches + Send + 'static,
        O: Operation<P>,
        O::Input: Send + 'static,
        O::Output: Send + 'static,
        O::Error: Send + Sync + 'static,
        D: Decoder<A, O::Input, G>,
        Q: Projector<O::Output, O::Error, D::Error, G>,
    {
        self.command = None;
        self.routes.clear();
        self.inventory.clear();
        self.registrations.push(Registration {
            path: path.into_iter().collect(),
            route: Arc::new(Route {
                binding,
                marker: PhantomData,
            }),
        });
        self
    }

    /// Validate and assemble the final clap grammar.
    ///
    /// # Errors
    ///
    /// Returns a deterministic route or argument conflict.
    pub fn build(mut self) -> Result<Self, BuildError> {
        let mut root = Node::default();
        for registration in &self.registrations {
            insert(&mut root, registration)?;
        }
        for namespace in &self.namespaces {
            apply_namespace(&mut root, namespace)?;
        }
        validate_aliases(&root, &[])?;

        let mut command = global_command::<G>(self.app.command());
        let global_keys = argument_keys(&command);
        validate_global_arguments(&root, &global_keys, &mut Vec::new())?;
        for (name, node) in &root.children {
            command = command.subcommand(node_command(name, node));
        }
        command = command
            .subcommand(completions_command(self.app.completions))
            .subcommand_required(true)
            .arg_required_else_help(true);

        self.routes = self
            .registrations
            .iter()
            .map(|registration| {
                (
                    registration.path.iter().map(ToString::to_string).collect(),
                    Arc::clone(&registration.route),
                )
            })
            .collect();
        self.inventory = inventory(&self.registrations);
        self.inventory.push(RouteInfo {
            selector: Selector {
                path: vec![COMPLETIONS.to_owned()],
            },
            operation_type_id: None,
            about: Some(self.app.completions.about),
            long_about: self.app.completions.long_about,
            aliases: Vec::new(),
            hidden: false,
        });
        self.inventory.sort_by(|left, right| left.selector.cmp(&right.selector));
        self.command = Some(command);
        Ok(self)
    }

    /// Return the assembled clap grammar.
    #[must_use]
    pub const fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }

    /// Return the deterministic route inventory.
    #[must_use]
    pub fn inventory(&self) -> &[RouteInfo] {
        &self.inventory
    }

    /// Parse and execute one argument vector.
    pub async fn execute<I, T>(&self, argv: I) -> CommandResponse
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString>,
    {
        let Some(command) = &self.command else {
            return CommandResponse::failure("command router has not been built\n", 1);
        };
        let mut argv: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
        if argv.is_empty() {
            argv.push(self.app.name.into());
        } else {
            argv[0] = self.app.name.into();
        }
        let matches = match command.clone().try_get_matches_from(argv) {
            Ok(matches) => matches,
            Err(error) => return clap_error(&error),
        };
        let (path, leaf_matches) = selected(&matches);
        if path == [COMPLETIONS] {
            return completion(command, leaf_matches);
        }
        let globals = match G::from_arg_matches(&matches) {
            Ok(globals) => globals,
            Err(error) => return clap_error(&error),
        };
        if let Some(response) = self.before_dispatch.as_ref().and_then(|hook| hook(&globals)) {
            return response;
        }
        match self.routes.get(&path) {
            Some(route) => route.dispatch(leaf_matches, &globals, &self.invoker).await,
            None => CommandResponse::failure("command route was not registered\n", 1),
        }
    }
}

/// Execute an explicit command router at the WASI CLI boundary.
///
/// Output is written before a non-zero response exits with its exact status.
///
/// # Errors
///
/// Returns failure when writing command output fails.
#[cfg(target_arch = "wasm32")]
pub async fn execute_wasi<P, G>(router: &Router<P, G>) -> Result<(), ()>
where
    P: Provider,
    G: Args + FromArgMatches + Send + Sync + 'static,
{
    use std::io::Write as _;

    let argv = wasip3::cli::environment::get_arguments();
    let response = router.execute(argv).await;
    if std::io::stdout().write_all(&response.stdout).is_err()
        || std::io::stderr().write_all(&response.stderr).is_err()
    {
        return Err(());
    }
    if response.exit == 0 {
        Ok(())
    } else {
        wasip3::cli::exit::exit_with_code(response.exit);
        Err(())
    }
}

fn global_command<G: Args>(command: Command) -> Command {
    let mut command = G::augment_args(command);
    let ids: Vec<_> = command.get_arguments().map(|argument| argument.get_id().clone()).collect();
    for id in ids {
        command = command.mut_arg(id, |argument| argument.global(true));
    }
    command
}

fn argument_keys(command: &Command) -> BTreeSet<String> {
    command
        .get_arguments()
        .flat_map(|argument| {
            [
                Some(argument.get_id().as_str().to_owned()),
                argument.get_long().map(|long| format!("--{long}")),
                argument.get_short().map(|short| format!("-{short}")),
            ]
        })
        .flatten()
        .collect()
}

fn insert<P: Provider, G>(
    root: &mut Node<P, G>, registration: &Registration<P, G>,
) -> Result<(), BuildError> {
    if registration.path.is_empty() {
        return Err(BuildError::EmptyPath);
    }
    if registration.path.iter().any(|segment| segment.is_empty()) {
        return Err(BuildError::EmptySegment);
    }
    let owned_path: Vec<_> = registration.path.iter().map(ToString::to_string).collect();
    if registration.path[0] == COMPLETIONS
        || registration.path.len() == 1 && registration.route.aliases().contains(&COMPLETIONS)
    {
        return Err(BuildError::ReservedCompletion(owned_path));
    }
    let mut node = root;
    for (index, segment) in registration.path.iter().enumerate() {
        if node.leaf.is_some() {
            return Err(BuildError::LeafNamespace(
                registration.path[..index].iter().map(ToString::to_string).collect(),
            ));
        }
        node = node.children.entry(segment).or_default();
    }
    if !node.children.is_empty() {
        return Err(BuildError::LeafNamespace(owned_path));
    }
    if node.leaf.is_some() {
        return Err(BuildError::DuplicatePath(owned_path));
    }
    node.leaf = Some(Arc::clone(&registration.route));
    Ok(())
}

fn apply_namespace<P: Provider, G>(
    root: &mut Node<P, G>, registration: &NamespaceRegistration,
) -> Result<(), BuildError> {
    let path: Vec<_> = registration.path.iter().map(ToString::to_string).collect();
    if registration.path.is_empty() {
        return Err(BuildError::EmptyPath);
    }
    if registration.path.iter().any(|segment| segment.is_empty()) {
        return Err(BuildError::EmptySegment);
    }
    if registration.path[0] == COMPLETIONS {
        return Err(BuildError::ReservedCompletion(path));
    }
    let mut node = root;
    for segment in &registration.path {
        let Some(child) = node.children.get_mut(segment) else {
            return Err(BuildError::UnknownNamespace(path));
        };
        node = child;
    }
    if node.leaf.is_some() {
        return Err(BuildError::NamespaceLeaf(path));
    }
    if node.namespace.is_some() {
        return Err(BuildError::DuplicateNamespace(path));
    }
    node.namespace = Some(registration.metadata);
    Ok(())
}

fn validate_aliases<P: Provider, G>(node: &Node<P, G>, path: &[String]) -> Result<(), BuildError> {
    let mut names = BTreeSet::new();
    for (name, child) in &node.children {
        if !names.insert(*name) {
            return Err(BuildError::AliasConflict(path.to_vec(), (*name).to_owned()));
        }
        if let Some(route) = &child.leaf {
            for alias in route.aliases() {
                if !names.insert(*alias) || node.children.contains_key(alias) {
                    return Err(BuildError::AliasConflict(path.to_vec(), (*alias).to_owned()));
                }
            }
        }
    }
    for (name, child) in &node.children {
        let mut child_path = path.to_vec();
        child_path.push((*name).to_owned());
        validate_aliases(child, &child_path)?;
    }
    Ok(())
}

fn validate_global_arguments<P: Provider, G>(
    node: &Node<P, G>, globals: &BTreeSet<String>, path: &mut Vec<String>,
) -> Result<(), BuildError> {
    if let Some(route) = &node.leaf
        && let Some(collision) =
            route.argument_keys().into_iter().find(|argument| globals.contains(argument))
    {
        return Err(BuildError::GlobalArgument(path.clone(), collision));
    }
    for (name, child) in &node.children {
        path.push((*name).to_owned());
        validate_global_arguments(child, globals, path)?;
        path.pop();
    }
    Ok(())
}

fn node_command<P: Provider, G>(name: &'static str, node: &Node<P, G>) -> Command {
    let mut command =
        node.leaf.as_ref().map_or_else(|| Command::new(name), |route| route.command(name));
    if let Some(namespace) = node.namespace {
        if let Some(about) = namespace.about {
            command = command.about(about);
        }
        if let Some(long_about) = namespace.long_about {
            command = command.long_about(long_about);
        }
    }
    for (child_name, child) in &node.children {
        command = command.subcommand(node_command(child_name, child));
    }
    command
}

fn completions_command(metadata: Completions) -> Command {
    let mut command = Command::new(COMPLETIONS).about(metadata.about);
    if let Some(long_about) = metadata.long_about {
        command = command.long_about(long_about);
    }
    command.arg(Arg::new("shell").required(true).value_parser(clap::value_parser!(Shell)))
}

fn inventory<P: Provider, G>(registrations: &[Registration<P, G>]) -> Vec<RouteInfo> {
    registrations
        .iter()
        .map(|registration| RouteInfo {
            selector: Selector {
                path: registration.path.iter().map(ToString::to_string).collect(),
            },
            operation_type_id: Some(registration.route.operation_type_id()),
            about: registration.route.about(),
            long_about: registration.route.long_about(),
            aliases: registration.route.aliases().to_vec(),
            hidden: registration.route.hidden(),
        })
        .collect()
}

fn selected(mut matches: &ArgMatches) -> (Vec<String>, &ArgMatches) {
    let mut path = Vec::new();
    while let Some((name, child)) = matches.subcommand() {
        path.push(name.to_owned());
        matches = child;
    }
    (path, matches)
}

fn completion(command: &Command, matches: &ArgMatches) -> CommandResponse {
    let Some(shell) = matches.get_one::<Shell>("shell").copied() else {
        return CommandResponse::failure("completion shell was not parsed\n", 1);
    };
    let mut command = command.clone();
    let name = command.get_name().to_owned();
    let mut output = Vec::new();
    clap_complete::generate(shell, &mut command, name, &mut output);
    CommandResponse::success(output)
}

fn clap_error(error: &clap::Error) -> CommandResponse {
    let rendered = error.render().to_string().into_bytes();
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => CommandResponse::success(rendered),
        _ => CommandResponse::failure(rendered, 2),
    }
}
