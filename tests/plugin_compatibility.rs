use std::path::{Path, PathBuf};
use std::sync::Arc;

use vim_script::host::{Capability, CommandDefinition, HostRuntime};
use vim_script::mock_editor::MockEditor;
use vim_script::plugin::{RuntimePath, ScriptLoader};
use vim_script::runtime::Value;

fn plugin_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugins/basic")
}

#[test]
fn discovers_initializes_and_executes_a_tier_one_plugin() {
    let editor = MockEditor::default();
    let mut host = HostRuntime::new(Arc::new(editor.clone()));
    host.capabilities.grant(Capability::FileSystemWrite);
    host.register_command(CommandDefinition {
        name: "write".into(),
        minimum_abbreviation: 1,
        accepts_bang: true,
        accepts_range: false,
        accepts_count: false,
        accepts_register: false,
        required_capabilities: vec![Capability::FileSystemWrite],
    });

    let mut loader = ScriptLoader::with_host(RuntimePath::new([plugin_root()]), host);
    let report = loader.load_startup_plugins();
    assert!(report.is_compatible(), "{:#?}", report.failures);
    assert_eq!(report.discovered.len(), 2);
    assert_eq!(report.loaded.len(), 2);
    assert_eq!(
        loader.globals.get("g:basic_plugin_loaded"),
        Some(&Value::Integer(1))
    );
    assert_eq!(
        loader.globals.get("g:first_private_value"),
        Some(&Value::Integer(11))
    );
    assert_eq!(
        loader.globals.get("g:second_private_value"),
        Some(&Value::Integer(22))
    );

    // Each sourced file gets a distinct script namespace.
    assert_eq!(
        loader.globals.get("s0:private_value"),
        Some(&Value::Integer(11))
    );
    assert_eq!(
        loader.globals.get("s1:private_value"),
        Some(&Value::Integer(22))
    );

    let autoload = loader
        .autoload_for("demo#util#answer")
        .expect("autoload entry");
    assert!(autoload.ends_with("autoload/demo/util.vim"));

    let state = editor.snapshot().unwrap();
    assert_eq!(state.write_count, 1);
    assert_eq!(state.command_log.len(), 1);

    // Startup loading is idempotent.
    let second_report = loader.load_startup_plugins();
    assert!(second_report.loaded.is_empty());
    assert_eq!(editor.snapshot().unwrap().write_count, 1);
}

#[test]
fn reports_plugin_failures_without_losing_successful_scripts() {
    let temporary =
        std::env::temp_dir().join(format!("vim-script-plugin-report-{}", std::process::id()));
    let plugin = temporary.join("plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("good.vim"), "let g:good_loaded = 1\n").unwrap();
    std::fs::write(plugin.join("bad.vim"), "if 1\nlet g:never = 1\n").unwrap();

    let mut loader = ScriptLoader::new(RuntimePath::new([temporary.clone()]));
    let report = loader.load_startup_plugins();
    assert_eq!(report.discovered.len(), 2);
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(
        loader.globals.get("g:good_loaded"),
        Some(&Value::Integer(1))
    );

    std::fs::remove_dir_all(temporary).unwrap();
}
