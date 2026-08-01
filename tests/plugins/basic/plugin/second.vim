" A second script proves that s: state does not collide across files.
let s:private_value = 22
let g:second_private_value = s:private_value
