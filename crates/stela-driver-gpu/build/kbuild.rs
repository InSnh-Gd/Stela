//! The object lists a driver's Kbuild makefiles compose, read with the subset
//! of GNU Make they use. Anything outside that subset is refused, even in an
//! inactive conditional, rather than guessed at.
//!
//! Accepted lines: comments; backslash continuations; `include $(src)/<path>`;
//! `ifdef CONFIG_<X>`, `ifeq ($(CONFIG_<X>),y)`, `ifneq ($(CONFIG_<X>),y)` and
//! `endif`; `ccflags-y` and `ccflags-$(CONFIG_<X>)` holding nothing but `-I`
//! directories under `$(src)`, `$(srctree)` or `$(obj)`; and `<name>-y` or
//! `<name>-$(CONFIG_<X>)` assigned with `:=`, `+=` or `=` from object paths
//! and `$(<name>-y)` references. A `:=` variable expands its references when
//! assigned, as Make's simply expanded variables do; a reference in a
//! recursively expanded variable is refused.
//!
//! A line the subset does not cover and no object list depends on — compiler
//! warnings, host programs, generated-file rules, header tests — is recorded
//! in [`Kbuild::ignored`] instead. Every port asserts the recorded lines are
//! the ones a human reviewed, so nothing is dropped silently.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Bounds on what one driver's makefiles may hold.
const MAX_FILES: usize = 128;
const MAX_DEPTH: usize = 8;
const MAX_BYTES: usize = 1 << 20;

/// The `CONFIG_` symbols of a Kconfig header: `#define CONFIG_<X> <value>`.
pub struct Config(BTreeMap<String, String>);

impl Config {
    pub fn parse(header: &str) -> Result<Self, String> {
        let mut symbols = BTreeMap::new();
        for line in header.lines() {
            let Some(definition) = line.strip_prefix("#define ") else {
                continue;
            };
            let (name, value) = definition.split_once(' ').unwrap_or((definition, ""));
            if name.starts_with("CONFIG_")
                && symbols
                    .insert(name.to_owned(), value.trim().to_owned())
                    .is_some()
            {
                return Err(format!("{name} is defined twice"));
            }
        }
        Ok(Self(symbols))
    }

    /// Whether a `bool` or `tristate` symbol is `y`. A module (`m`) selection
    /// and a non-boolean value are refused: Stela builds no modules.
    fn enabled(&self, name: &str) -> Result<bool, String> {
        if self.0.contains_key(&format!("{name}_MODULE")) {
            return Err(format!("{name} selects a module"));
        }
        match self.0.get(name).map(String::as_str) {
            None => Ok(false),
            Some("1") => Ok(true),
            Some(value) => Err(format!("{name} is {value}, not a boolean")),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Flavor {
    Simple,
    Recursive,
}

/// An include directory a driver's makefiles add, as Make spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Include {
    /// `-I$(src)[/<path>]`, below the driver's own source directory.
    Source(PathBuf),
    /// `-I$(srctree)/<path>`, below the Linux tree's root.
    Tree(PathBuf),
    /// `-I$(obj)`, the build directory generated headers are written to.
    Object,
}

/// What a driver's makefiles define: its object lists and include paths.
#[derive(Default)]
pub struct Kbuild {
    /// The source directory `read` was given, which `$(src)` names.
    source: PathBuf,
    variables: BTreeMap<String, (Flavor, Vec<String>)>,
    /// `ccflags-y` include directories, in order.
    pub includes: Vec<Include>,
    /// Every makefile read, in order.
    pub files: Vec<PathBuf>,
    /// Statements outside the subset that no object list depends on.
    pub ignored: Vec<String>,
    /// The composite names the caller wants object lists for. A variable with
    /// any other name holds an intermediate another variable references.
    composites: BTreeSet<String>,
}

impl Kbuild {
    /// Reads `file` in `source` and the makefiles it includes, which name
    /// paths relative to `source` as `$(src)`. Only `composites` keeps an
    /// object list; every other variable is ignored and recorded.
    pub fn read(
        source: &Path,
        file: &str,
        config: &Config,
        composites: &[&str],
    ) -> Result<Self, String> {
        let mut kbuild = Self {
            source: source.to_owned(),
            composites: composites.iter().map(|name| (*name).to_owned()).collect(),
            ..Self::default()
        };
        kbuild.makefile(Path::new(file), config, 0)?;
        Ok(kbuild)
    }

    /// The objects of `<name>-y`, as paths below the source directory.
    pub fn objects(&self, name: &str) -> Result<&[String], String> {
        self.variables
            .get(&format!("{name}-y"))
            .map(|(_, objects)| objects.as_slice())
            .ok_or_else(|| format!("Kbuild defines no {name}-y"))
    }

    fn makefile(&mut self, file: &Path, config: &Config, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH || self.files.len() == MAX_FILES {
            return Err(format!("{} nests past Kbuild's bounds", file.display()));
        }
        let path = self.source.join(file);
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if text.len() > MAX_BYTES {
            return Err(format!("{} is too large", path.display()));
        }
        self.files.push(path.clone());
        let mut active = vec![true];
        let mut pending: Option<(usize, String)> = None;
        for (number, raw) in text.lines().enumerate() {
            let continued = raw.ends_with('\\');
            let fragment = raw.strip_suffix('\\').unwrap_or(raw).trim();
            let statement = pending.get_or_insert_with(|| (number, String::new()));
            if !statement.1.is_empty() {
                statement.1.push(' ');
            }
            statement.1.push_str(fragment);
            if continued {
                continue;
            }
            let (start, line) = pending.take().expect("a pending statement");
            // Make reads tabs, spaces and newlines alike between words.
            let line = line.replace('\t', " ");
            self.statement(config, &mut active, &path, start + 1, &line, depth)?;
        }
        if pending.is_some() {
            return Err(format!("{} ends inside a continuation", path.display()));
        }
        if active.len() != 1 {
            return Err(format!("{}: ifdef without endif", path.display()));
        }
        Ok(())
    }

    fn statement(
        &mut self,
        config: &Config,
        active: &mut Vec<bool>,
        path: &Path,
        number: usize,
        line: &str,
        depth: usize,
    ) -> Result<(), String> {
        let at = || format!("{}:{number}", path.display());
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return Ok(());
        }
        if let Some(symbol) = line.strip_prefix("ifdef ") {
            let enabled = config
                .enabled(symbol.trim())
                .map_err(|error| format!("{}: {error}", at()))?;
            active.push(active[active.len() - 1] && enabled);
            return Ok(());
        }
        for (keyword, matches) in [("ifeq", true), ("ifneq", false)] {
            let Some(expression) = line
                .strip_prefix(keyword)
                .map(str::trim)
                .and_then(|rest| rest.strip_prefix('('))
                .and_then(|rest| rest.strip_suffix(')'))
            else {
                continue;
            };
            let (left, right) = expression
                .split_once(',')
                .ok_or_else(|| format!("{}: unsupported {keyword} {expression}", at()))?;
            let symbol = left
                .trim()
                .strip_prefix("$(")
                .and_then(|left| left.strip_suffix(')'))
                .filter(|symbol| symbol.starts_with("CONFIG_"))
                .ok_or_else(|| format!("{}: unsupported {keyword} {expression}", at()))?;
            if right.trim() != "y" {
                return Err(format!("{}: unsupported {keyword} {expression}", at()));
            }
            // A `bool` symbol compares equal to `y` exactly when it is set.
            let enabled = config
                .enabled(symbol)
                .map_err(|error| format!("{}: {error}", at()))?;
            active.push(active[active.len() - 1] && (enabled == matches));
            return Ok(());
        }
        if line == "endif" {
            if active.len() == 1 {
                return Err(format!("{}: endif without ifdef", at()));
            }
            active.pop();
            return Ok(());
        }
        let statement = Statement::parse(line).map_err(|error| format!("{}: {error}", at()))?;
        if !active[active.len() - 1] {
            return Ok(());
        }
        match statement {
            Statement::Include(relative) => {
                self.makefile(Path::new(relative), config, depth + 1)?;
            }
            Statement::Flags { condition, value } => {
                if let Some(symbol) = condition
                    && !config
                        .enabled(symbol)
                        .map_err(|error| format!("{}: {error}", at()))?
                {
                    return Ok(());
                }
                match flag_directories(value) {
                    Some(directories) => {
                        for directory in directories {
                            self.includes.push(
                                include(directory).map_err(|error| format!("{}: {error}", at()))?,
                            );
                        }
                    }
                    None => self.ignored.push(line.to_owned()),
                }
            }
            Statement::Assign {
                name,
                condition,
                replaces,
                words,
            } => {
                if !self.composites.contains(name) {
                    self.ignored.push(line.to_owned());
                    return Ok(());
                }
                if let Some(symbol) = condition
                    && !config
                        .enabled(symbol)
                        .map_err(|error| format!("{}: {error}", at()))?
                {
                    return Ok(());
                }
                self.assign(name, replaces, &words)
                    .map_err(|error| format!("{}: {error}", at()))?;
            }
            Statement::Ignored => self.ignored.push(line.to_owned()),
        }
        Ok(())
    }

    fn assign(&mut self, name: &str, replaces: bool, words: &[&str]) -> Result<(), String> {
        let key = format!("{name}-y");
        let flavor = match (replaces, self.variables.get(&key)) {
            (true, _) => Flavor::Simple,
            (false, Some((flavor, _))) => *flavor,
            (false, None) => Flavor::Recursive,
        };
        let mut values = Vec::new();
        for word in words {
            if let Some(reference) = word
                .strip_prefix("$(")
                .and_then(|word| word.strip_suffix(')'))
            {
                if flavor == Flavor::Recursive {
                    return Err(format!("{key} refers to {reference} without :="));
                }
                let (_, referenced) = self
                    .variables
                    .get(reference)
                    .ok_or_else(|| format!("{key} refers to undefined {reference}"))?;
                values.extend(referenced.iter().cloned());
            } else if word.ends_with(".o") && !word.contains('$') && !word.contains("..") {
                values.push((*word).to_owned());
            } else {
                return Err(format!("{key} holds {word}, which is no object"));
            }
        }
        let entry = self.variables.entry(key).or_insert((flavor, Vec::new()));
        if replaces {
            entry.1.clear();
        }
        entry.0 = flavor;
        entry.1.extend(values);
        Ok(())
    }
}

/// One line of the accepted subset.
enum Statement<'a> {
    Include(&'a str),
    Flags {
        condition: Option<&'a str>,
        value: &'a str,
    },
    Assign {
        name: &'a str,
        condition: Option<&'a str>,
        replaces: bool,
        words: Vec<&'a str>,
    },
    Ignored,
}

impl<'a> Statement<'a> {
    fn parse(line: &'a str) -> Result<Self, String> {
        if let Some(path) = line.strip_prefix("include $(src)/") {
            return relative(path).map(Statement::Include);
        }
        let Some((target, replaces, value)) = assignment(line) else {
            return Ok(Statement::Ignored);
        };
        let Some((name, selector)) = target.rsplit_once('-') else {
            return Ok(Statement::Ignored);
        };
        let condition = match selector {
            "y" => None,
            selector => match selector
                .strip_prefix("$(")
                .and_then(|selector| selector.strip_suffix(')'))
                .filter(|symbol| symbol.starts_with("CONFIG_"))
            {
                Some(symbol) => Some(symbol),
                None => return Ok(Statement::Ignored),
            },
        };
        // Compiler warnings and include directories share a variable name.
        if name.ends_with("ccflags") {
            return Ok(Statement::Flags { condition, value });
        }
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Ok(Statement::Ignored);
        }
        Ok(Statement::Assign {
            name,
            condition,
            replaces,
            words: value.split_whitespace().collect(),
        })
    }
}

/// The target, whether it replaces the variable, and the value of an
/// assignment, or `None` for a make rule, a recipe or a bare line.
fn assignment(line: &str) -> Option<(&str, bool, &str)> {
    for (operator, replaces) in [(" := ", true), (" += ", false), (" = ", false)] {
        if let Some((target, value)) = line.split_once(operator) {
            return Some((target.trim(), replaces, value));
        }
    }
    None
}

/// The `-I` directories of a flags value, or `None` when it holds any other
/// flag, which no object list depends on.
fn flag_directories(value: &str) -> Option<Vec<&str>> {
    let mut directories = Vec::new();
    let mut words = value.split_whitespace();
    while let Some(word) = words.next() {
        // `-I` on its own takes the next word as its directory.
        directories.push(match word.strip_prefix("-I") {
            Some("") => words.next()?,
            Some(directory) => directory,
            None => return None,
        });
    }
    Some(directories)
}

/// The include directory one `-I` flag of a driver's makefiles names.
fn include(flag: &str) -> Result<Include, String> {
    if flag == "$(obj)" {
        return Ok(Include::Object);
    }
    let (below, tree): (&str, fn(PathBuf) -> Include) =
        if let Some(below) = flag.strip_prefix("$(src)") {
            (below, Include::Source)
        } else if let Some(below) = flag.strip_prefix("$(srctree)") {
            (below, Include::Tree)
        } else {
            return Err(format!(
                "{flag} is no include directory below $(src), $(srctree) or $(obj)"
            ));
        };
    let below = below
        .strip_prefix('/')
        .unwrap_or(below)
        .trim_end_matches('/');
    // `-I$(src)` alone names the driver's own directory.
    let below = match below.is_empty() {
        true => PathBuf::new(),
        false => PathBuf::from(relative(below)?),
    };
    Ok(tree(below))
}

/// A path below the directory named by `$(src)` or `$(srctree)`.
fn relative(path: &str) -> Result<&str, String> {
    if path.starts_with('/') || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(format!("{path} leaves the driver's tree"));
    }
    Ok(path)
}
