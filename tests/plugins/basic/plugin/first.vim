" Tier-1 plugin fixture: global initialization, script-local state, editor command.
let g:basic_plugin_loaded = 1
let s:private_value = 11
let g:first_private_value = s:private_value
:write
