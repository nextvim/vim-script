use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ast::ExCommand;
use crate::runtime::{HostObjectId, RuntimeError, RuntimeErrorKind, RuntimeResult, Value, Vm};

pub type HostFuture = Pin<Box<dyn Future<Output = RuntimeResult<Value>> + Send + 'static>>;

/// Application boundary for asynchronous operations. Requests own all data so
/// returned futures can outlive a VM quantum and move to an I/O executor.
pub trait Host: Send + Sync + 'static {
    fn call(&self, request: HostRequest) -> HostFuture;

    fn execute_command(&self, request: CommandRequest) -> HostFuture {
        Box::pin(async move {
            Err(RuntimeError::coded(
                "E492",
                RuntimeErrorKind::InvalidCommand,
                format!("host does not implement command {}", request.command.name),
            ))
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostRequest {
    pub target: HostTarget,
    pub function: String,
    pub arguments: Vec<Value>,
    pub context: HostContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostTarget {
    Global,
    Namespace(String),
    Object(HostObjectId),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostContext {
    pub script_name: Option<String>,
    pub current_buffer: Option<u64>,
    pub current_window: Option<u64>,
    pub current_tab: Option<u64>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    Editor,
    BufferRead,
    BufferWrite,
    Window,
    Settings,
    FileSystemRead,
    FileSystemWrite,
    Network,
    ClipboardRead,
    ClipboardWrite,
    Terminal,
    Process,
    UserInterface,
    Custom(String),
}

#[derive(Clone, Debug, Default)]
pub struct CapabilitySet {
    pub granted: HashSet<Capability>,
}

impl CapabilitySet {
    pub fn allows(&self, capability: &Capability) -> bool {
        self.granted.contains(capability)
    }
    pub fn grant(&mut self, capability: Capability) -> bool {
        self.granted.insert(capability)
    }
    pub fn revoke(&mut self, capability: &Capability) -> bool {
        self.granted.remove(capability)
    }
    pub fn allows_all(&self, capabilities: &[Capability]) -> bool {
        capabilities
            .iter()
            .all(|capability| self.allows(capability))
    }
}

impl<const N: usize> From<[Capability; N]> for CapabilitySet {
    fn from(capabilities: [Capability; N]) -> Self {
        Self {
            granted: capabilities.into_iter().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arity {
    Exact(u16),
    Range { min: u16, max: u16 },
    Variadic { min: u16 },
}

impl Arity {
    pub fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exact(expected) => count == expected as usize,
            Self::Range { min, max } => (min as usize..=max as usize).contains(&count),
            Self::Variadic { min } => count >= min as usize,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HostFunctionRegistration {
    pub name: String,
    pub target: HostTarget,
    pub arity: Arity,
    pub required_capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Default)]
pub struct HostFunctionRegistry {
    pub functions: HashMap<String, HostFunctionRegistration>,
}

impl HostFunctionRegistry {
    pub fn register(&mut self, registration: HostFunctionRegistration) {
        self.functions
            .insert(registration.name.clone(), registration);
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.functions.keys().map(String::as_str)
    }
    pub fn get(&self, name: &str) -> Option<&HostFunctionRegistration> {
        self.functions.get(name)
    }
}

#[derive(Clone)]
pub struct HostRuntime {
    pub host: Arc<dyn Host>,
    pub capabilities: CapabilitySet,
    pub functions: HostFunctionRegistry,
    pub commands: CommandRegistry,
}

impl std::fmt::Debug for HostRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostRuntime")
            .field("capabilities", &self.capabilities)
            .field("functions", &self.functions)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

impl HostRuntime {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self {
            host,
            capabilities: CapabilitySet::default(),
            functions: HostFunctionRegistry::default(),
            commands: CommandRegistry::default(),
        }
    }

    pub fn register_function(
        &mut self,
        name: impl Into<String>,
        arity: Arity,
        required_capabilities: Vec<Capability>,
    ) {
        let name = name.into();
        self.functions.register(HostFunctionRegistration {
            name: name.clone(),
            target: HostTarget::Global,
            arity,
            required_capabilities,
        });
    }

    pub fn register_command(&mut self, definition: CommandDefinition) {
        self.commands.register(definition);
    }

    pub fn install_globals(&self, vm: &mut Vm) {
        for name in self.functions.names() {
            let function = Value::HostFunction(Arc::from(name));
            vm.globals.insert(format!(":{name}"), function.clone());
            vm.globals.insert(format!("g:{name}"), function);
        }
    }

    pub fn dispatch(&self, mut request: HostRequest) -> RuntimeResult<HostFuture> {
        let registration = self.functions.get(&request.function).ok_or_else(|| {
            RuntimeError::coded(
                "E117",
                RuntimeErrorKind::NameError,
                format!("unknown host function: {}", request.function),
            )
        })?;
        if !registration.arity.accepts(request.arguments.len()) {
            return Err(RuntimeError::coded(
                "E119",
                RuntimeErrorKind::ArityError,
                format!("invalid argument count for {}", request.function),
            ));
        }
        if let Some(missing) = registration
            .required_capabilities
            .iter()
            .find(|capability| !self.capabilities.allows(capability))
        {
            return Err(RuntimeError::coded(
                "E_PERM",
                RuntimeErrorKind::PermissionDenied,
                format!(
                    "host function {} requires capability {missing:?}",
                    request.function
                ),
            ));
        }
        request.target = registration.target.clone();
        Ok(self.host.call(request))
    }

    pub fn dispatch_command(&self, mut request: CommandRequest) -> RuntimeResult<HostFuture> {
        let definition = self.commands.resolve(&request.command.name)?;
        request.command.name = definition.name.clone();
        if request.command.bang && !definition.accepts_bang {
            return Err(RuntimeError::coded(
                "E477",
                RuntimeErrorKind::InvalidCommand,
                format!("{} does not accept !", definition.name),
            ));
        }
        if request.command.range.is_some() && !definition.accepts_range {
            return Err(RuntimeError::coded(
                "E481",
                RuntimeErrorKind::InvalidCommand,
                format!("{} does not accept a range", definition.name),
            ));
        }
        if !self
            .capabilities
            .allows_all(&definition.required_capabilities)
        {
            return Err(RuntimeError::coded(
                "E_PERM",
                RuntimeErrorKind::PermissionDenied,
                format!("command {} lacks required capabilities", definition.name),
            ));
        }
        Ok(self.host.execute_command(request))
    }
}

pub type CommandFuture = HostFuture;

#[derive(Clone, Debug, PartialEq)]
pub struct CommandRequest {
    pub command: ExCommand,
    pub context: HostContext,
}

#[derive(Clone, Debug)]
pub struct CommandDefinition {
    pub name: String,
    pub minimum_abbreviation: usize,
    pub accepts_bang: bool,
    pub accepts_range: bool,
    pub accepts_count: bool,
    pub accepts_register: bool,
    pub required_capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Default)]
pub struct CommandRegistry {
    pub commands: HashMap<String, CommandDefinition>,
}

impl CommandRegistry {
    pub fn register(&mut self, definition: CommandDefinition) {
        self.commands.insert(definition.name.clone(), definition);
    }
    pub fn resolve(&self, name: &str) -> RuntimeResult<&CommandDefinition> {
        if let Some(command) = self.commands.get(name) {
            return Ok(command);
        }
        let mut matches = self.commands.values().filter(|command| {
            name.len() >= command.minimum_abbreviation && command.name.starts_with(name)
        });
        let Some(command) = matches.next() else {
            return Err(RuntimeError::coded(
                "E492",
                RuntimeErrorKind::InvalidCommand,
                format!("not an editor command: {name}"),
            ));
        };
        if matches.next().is_some() {
            return Err(RuntimeError::coded(
                "E464",
                RuntimeErrorKind::InvalidCommand,
                format!("ambiguous command: {name}"),
            ));
        }
        Ok(command)
    }
}
