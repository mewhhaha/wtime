(module (import "m" "f" (func $f (param i32) (result i32))) (func (export "run") (result i32) i32.const 6 call $f))
