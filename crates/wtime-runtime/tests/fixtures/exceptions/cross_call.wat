(module (tag $e (param i32)) (func $raise i32.const 42 throw $e) (func (export "f") (result i32) (block (result i32) (try_table (catch $e 0) call $raise) i32.const 0)))
