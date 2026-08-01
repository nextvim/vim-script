use std::collections::HashMap;

use crate::ast::{MapMode, MappingOptions};
use crate::bytecode::BytecodeModule;
use crate::runtime::Value;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EventHandlerId(pub u64);

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub pattern: Option<String>,
    pub payload: HashMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct EventHandler {
    pub id: EventHandlerId,
    pub group: Option<String>,
    pub event: String,
    pub patterns: Vec<String>,
    pub module: BytecodeModule,
    pub once: bool,
    pub nested: bool,
}

#[derive(Clone, Debug, Default)]
pub struct EventBus {
    pub handlers: HashMap<String, Vec<EventHandler>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MappingId(pub u64);

#[derive(Clone, Debug)]
pub struct CompiledMapping {
    pub id: MappingId,
    pub modes: Vec<MapMode>,
    pub lhs: String,
    pub expansion: MappingExpansion,
    pub options: MappingOptions,
    pub buffer: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum MappingExpansion {
    Keys(String),
    Bytecode(BytecodeModule),
    NoOp,
}

#[derive(Clone, Debug, Default)]
pub struct KeymapStore {
    pub global: HashMap<(MapMode, String), CompiledMapping>,
    pub buffer_local: HashMap<u64, HashMap<(MapMode, String), CompiledMapping>>,
}

#[derive(Clone, Debug)]
pub struct OptionDefinition {
    pub name: String,
    pub short_name: Option<String>,
    pub kind: OptionKind,
    pub scope: OptionValueScope,
    pub default: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionKind {
    Boolean,
    Number,
    String,
    StringList,
    Flags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionValueScope {
    Global,
    Buffer,
    Window,
    GlobalBuffer,
    GlobalWindow,
}

#[derive(Clone, Debug, Default)]
pub struct OptionStore {
    pub definitions: HashMap<String, OptionDefinition>,
    pub global: HashMap<String, Value>,
    pub buffers: HashMap<u64, HashMap<String, Value>>,
    pub windows: HashMap<u64, HashMap<String, Value>>,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleCache {
    pub modules: HashMap<ModuleCacheKey, BytecodeModule>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModuleCacheKey {
    pub source_name: String,
    pub content_hash: u64,
    pub language_version: LanguageVersion,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LanguageVersion {
    Legacy,
    Vim9,
}
