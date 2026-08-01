use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::compiler::Compiler;
use crate::host::HostRuntime;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::resolver::{Resolver, ResolverConfig};
use crate::runtime::{RuntimeError, RuntimeErrorKind, Scheduler, Value, Vm};
use crate::source::{Diagnostic, SourceId, SourceMap};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ScriptId(pub u32);

#[derive(Clone, Debug, Default)]
pub struct RuntimePath {
    pub roots: Vec<PathBuf>,
}

impl RuntimePath {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }
    pub fn push(&mut self, root: impl Into<PathBuf>) {
        self.roots.push(root.into());
    }

    pub fn startup_plugins(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for root in &self.roots {
            collect_vim_files(&root.join("plugin"), false, &mut paths);
        }
        paths.sort();
        paths
    }

    pub fn autoload_files(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for root in &self.roots {
            collect_vim_files(&root.join("autoload"), true, &mut paths);
        }
        paths.sort();
        paths
    }

    pub fn find_autoload(&self, function: &str) -> Option<PathBuf> {
        let mut components: Vec<_> = function.split('#').collect();
        if components.len() < 2 {
            return None;
        }
        components.pop();
        let relative = components.join("/") + ".vim";
        self.roots
            .iter()
            .map(|root| root.join("autoload").join(&relative))
            .find(|path| path.is_file())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityStage {
    Discovery,
    Io,
    Lex,
    Parse,
    Resolve,
    Compile,
    Runtime,
}

#[derive(Clone, Debug)]
pub struct CompatibilityFailure {
    pub path: Option<PathBuf>,
    pub stage: CompatibilityStage,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Default)]
pub struct CompatibilityReport {
    pub discovered: Vec<PathBuf>,
    pub loaded: Vec<PathBuf>,
    pub failures: Vec<CompatibilityFailure>,
    pub unsupported_features: HashSet<String>,
}

impl CompatibilityReport {
    pub fn is_compatible(&self) -> bool {
        self.failures.is_empty()
    }
    pub fn record_unsupported(&mut self, feature: impl Into<String>) {
        self.unsupported_features.insert(feature.into());
    }
}

#[derive(Clone, Debug)]
pub struct LoadedScript {
    pub id: ScriptId,
    pub source: SourceId,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ScriptLoader {
    pub runtime_path: RuntimePath,
    pub sources: SourceMap,
    pub loaded_scripts: HashMap<PathBuf, LoadedScript>,
    pub autoload_index: HashMap<String, PathBuf>,
    pub globals: HashMap<String, Value>,
    pub host: Option<HostRuntime>,
    pub instruction_quantum: usize,
}

impl ScriptLoader {
    pub fn new(runtime_path: RuntimePath) -> Self {
        let mut loader = Self {
            runtime_path,
            sources: SourceMap::default(),
            loaded_scripts: HashMap::new(),
            autoload_index: HashMap::new(),
            globals: HashMap::new(),
            host: None,
            instruction_quantum: 10_000,
        };
        loader.rebuild_autoload_index();
        loader
    }

    pub fn with_host(runtime_path: RuntimePath, host: HostRuntime) -> Self {
        let mut loader = Self::new(runtime_path);
        loader.host = Some(host);
        loader
    }

    pub fn rebuild_autoload_index(&mut self) {
        self.autoload_index.clear();
        for path in self.runtime_path.autoload_files() {
            if let Some(prefix) = autoload_prefix(&self.runtime_path, &path) {
                self.autoload_index.entry(prefix).or_insert(path);
            }
        }
    }

    pub fn autoload_for(&self, function: &str) -> Option<PathBuf> {
        self.autoload_index
            .iter()
            .filter(|(prefix, _)| function.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, path)| path.clone())
            .or_else(|| self.runtime_path.find_autoload(function))
    }

    pub fn load_startup_plugins(&mut self) -> CompatibilityReport {
        let paths = self.runtime_path.startup_plugins();
        let mut report = CompatibilityReport {
            discovered: paths.clone(),
            ..CompatibilityReport::default()
        };
        for path in paths {
            if self.loaded_scripts.contains_key(&path) {
                continue;
            }
            match self.load_script(&path) {
                Ok(()) => report.loaded.push(path),
                Err(failure) => report.failures.push(failure),
            }
        }
        report
    }

    pub fn load_script(&mut self, path: &Path) -> Result<(), CompatibilityFailure> {
        let canonical = path.canonicalize().map_err(|error| {
            failure(path, CompatibilityStage::Io, error.to_string(), Vec::new())
        })?;
        if self.loaded_scripts.contains_key(&canonical) {
            return Ok(());
        }
        let text = fs::read_to_string(&canonical).map_err(|error| {
            failure(
                &canonical,
                CompatibilityStage::Io,
                error.to_string(),
                Vec::new(),
            )
        })?;
        let source = self.sources.add_path(canonical.clone(), text.clone());
        let script = ScriptId(source.0);
        let lexed = Lexer::new(source, &text).lex();
        if !lexed.diagnostics.is_empty() {
            return Err(failure(
                &canonical,
                CompatibilityStage::Lex,
                "lexing failed",
                lexed.diagnostics,
            ));
        }
        let parsed = Parser::new(&lexed.tokens).parse();
        if !parsed.diagnostics.is_empty() {
            return Err(failure(
                &canonical,
                CompatibilityStage::Parse,
                "parsing failed",
                parsed.diagnostics,
            ));
        }
        let mut config = ResolverConfig::default();
        if let Some(host) = &self.host {
            config
                .builtins
                .extend(host.functions.names().map(str::to_owned));
        }
        let resolved =
            Resolver::new(config).resolve(parsed.program.expect("parser always returns a program"));
        if !resolved.diagnostics.is_empty() {
            return Err(failure(
                &canonical,
                CompatibilityStage::Resolve,
                "semantic resolution failed",
                resolved.diagnostics,
            ));
        }
        let compiled =
            Compiler::new(&resolved.program.expect("resolver always returns a program")).compile();
        if !compiled.diagnostics.is_empty() {
            return Err(failure(
                &canonical,
                CompatibilityStage::Compile,
                "compilation failed",
                compiled.diagnostics,
            ));
        }
        let vm = Vm::with_globals(
            compiled.module.expect("compiler always returns a module"),
            self.globals.clone(),
        )
        .map_err(|error| runtime_failure(&canonical, error))?;
        let mut scheduler = Scheduler::new(self.instruction_quantum);
        if let Some(host) = self.host.clone() {
            scheduler.set_host(host);
        }
        let task = scheduler
            .spawn(vm)
            .map_err(|error| runtime_failure(&canonical, error))?;
        scheduler
            .run_until_complete(task)
            .map_err(|error| runtime_failure(&canonical, error))?;
        self.globals = scheduler
            .task(task)
            .expect("completed task exists")
            .vm
            .globals
            .clone();
        self.loaded_scripts.insert(
            canonical.clone(),
            LoadedScript {
                id: script,
                source,
                path: canonical,
            },
        );
        Ok(())
    }
}

fn collect_vim_files(directory: &Path, recursive: bool, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() && recursive {
            collect_vim_files(&path, true, output);
        } else if path.extension().is_some_and(|extension| extension == "vim") {
            output.push(path);
        }
    }
}

fn autoload_prefix(runtime_path: &RuntimePath, path: &Path) -> Option<String> {
    runtime_path.roots.iter().find_map(|root| {
        let relative = path.strip_prefix(root.join("autoload")).ok()?;
        let mut components: Vec<_> = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        let last = components.pop()?;
        components.push(last.strip_suffix(".vim")?.to_owned());
        Some(components.join("#") + "#")
    })
}

fn failure(
    path: &Path,
    stage: CompatibilityStage,
    message: impl Into<String>,
    diagnostics: Vec<Diagnostic>,
) -> CompatibilityFailure {
    CompatibilityFailure {
        path: Some(path.to_owned()),
        stage,
        message: message.into(),
        diagnostics,
    }
}
fn runtime_failure(path: &Path, error: RuntimeError) -> CompatibilityFailure {
    failure(
        path,
        CompatibilityStage::Runtime,
        format!(
            "{}{}",
            error
                .code
                .as_deref()
                .map_or(String::new(), |code| format!("{code}: ")),
            error.message
        ),
        Vec::new(),
    )
}

pub fn missing_feature(name: impl Into<String>) -> RuntimeError {
    RuntimeError::coded(
        "E_NOTIMPL",
        RuntimeErrorKind::HostError,
        format!("unsupported plugin feature: {}", name.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_autoload_function_names_to_paths() {
        let root = PathBuf::from("/runtime");
        let runtime = RuntimePath::new([root.clone()]);
        assert_eq!(runtime.find_autoload("example#util#run"), None);
        assert!(
            autoload_prefix(&runtime, &root.join("autoload/example/util.vim"))
                .is_some_and(|prefix| prefix == "example#util#")
        );
    }
}
