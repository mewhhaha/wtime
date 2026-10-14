(module (import "m" "raise" (func $raise)) (import "m" "e" (tag (param i32))) (func (export "f") (result i32) (block (result i32) (try_table (catch 0 0) call $raise) i32.const 0)))
