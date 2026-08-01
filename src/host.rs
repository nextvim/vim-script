use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::runtime::{HostObjectId, RuntimeResult, Value};

pub type HostFuture<'a> = Pin<Box<dyn Future<Output = RuntimeResult<Value>> + Send + 'a>>;

pub trait Host: Send {
    fn call<'a>(&'a mut self, call: HostCall<'a>) -> HostFuture<'a>;
}

#[derive(Clone, Debug)]
pub struct HostCall<'a> {
    pub target: HostTarget<'a>,
    pub function: &'a str,
    pub arguments: &'a [Value],
    pub context: HostContext,
}

#[derive(Clone, Copy, Debug)]
pub enum HostTarget<'a> {
    Global,
    Namespace(&'a str),
    Object(HostObjectId),
}

#[derive(Clone, Debug, Default)]
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
}

pub type NativeFuture = Pin<Box<dyn Future<Output = RuntimeResult<Value>> + Send + 'static>>;
pub type NativeFunction = Arc<dyn Fn(Vec<Value>) -> NativeFuture + Send + Sync>;

#[derive(Clone)]
pub struct FunctionRegistration {
    pub name: String,
    pub function: NativeFunction,
    pub arity: Arity,
    pub required_capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arity {
    Exact(u16),
    Range { min: u16, max: u16 },
    Variadic { min: u16 },
}

#[derive(Clone, Default)]
pub struct FunctionRegistry {
    pub functions: HashMap<String, FunctionRegistration>,
}

pub type CommandFuture = Pin<Box<dyn Future<Output = RuntimeResult<Value>> + Send + 'static>>;
pub type CommandHandler = Arc<dyn Fn(CommandInvocation) -> CommandFuture + Send + Sync>;

#[derive(Clone, Debug)]
pub struct CommandInvocation {
    pub name: String,
    pub bang: bool,
    pub arguments: String,
    pub range: Option<(i64, i64)>,
    pub count: Option<u64>,
    pub register: Option<char>,
}

#[derive(Clone)]
pub struct CommandRegistration {
    pub name: String,
    pub handler: CommandHandler,
    pub required_capabilities: Vec<Capability>,
}

#[derive(Clone, Default)]
pub struct CommandRegistry {
    pub commands: HashMap<String, CommandRegistration>,
}
