//! Pure text rendering: `gen.rs`, the `[[example]]` list, and dep-info.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::Program;

/// One built component: the program plus its artifact path.
pub struct Artifact<'a> {
    pub program: &'a Program,
    pub path: String,
}

/// Renders `gen.rs`: one `pub const <CONSTANT>: &str` per artifact, then one
/// `foreach_<group>!` completeness macro per group (an ungrouped artifact —
/// an extra package — gets a constant and no arm).
pub fn render_gen(header: &str, artifacts: &[Artifact<'_>]) -> String {
    let mut generated = String::from(header);
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for artifact in artifacts {
        let name = artifact.program.name.as_str();
        writeln!(
            generated,
            "/// Path to the compiled `{name}` guest component.\npub const {}: &str = {:?};",
            artifact.program.constant, artifact.path
        )
        .expect("writing to a String");
        if !artifact.program.group.is_empty() {
            groups.entry(&artifact.program.group).or_default().push(name);
        }
    }

    for (group, names) in groups {
        let expect = "        #[expect(unused_imports, reason = \"asserts the test exists\")]\n";
        let arms = names.iter().fold(String::new(), |mut arms, name| {
            let _ = writeln!(arms, "{expect}        use self::{name} as _;");
            arms
        });
        write!(
            generated,
            "/// Asserts an identically named test exists at the invocation site for \
             every `{group}` guest program; a program without one fails to \
             compile.\n#[macro_export]\nmacro_rules! \
             foreach_{group} {{\n    () => {{\n{arms}    }};\n}}\n"
        )
        .expect("writing to a String");
    }
    generated
}

/// Renders the manifest with everything after `marker` replaced by one
/// `[[example]]` stanza per program; `None` when the manifest lacks the marker.
pub fn refresh_examples(current: &str, marker: &str, programs: &[Program]) -> Option<String> {
    let (header, _) = current.split_once(marker)?;
    let mut next = String::from(header);
    next.push_str(marker);
    for program in programs {
        write!(
            next,
            "\n[[example]]\nname = \"{}\"\npath = \"{}\"\ncrate-type = [\"cdylib\"]\n",
            program.name, program.source
        )
        .expect("writing to a String");
    }
    Some(next)
}

/// The prerequisite paths in a cargo dep-info file, unescaping `\ `.
pub fn dep_info_paths(contents: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in contents.lines() {
        let Some((_, deps)) = line.split_once(": ") else {
            continue;
        };
        let mut current = String::new();
        for part in deps.split(' ') {
            if let Some(prefix) = part.strip_suffix('\\') {
                current.push_str(prefix);
                current.push(' ');
            } else if !part.is_empty() {
                current.push_str(part);
                paths.push(std::mem::take(&mut current));
            }
        }
    }
    paths
}
