(module (type $f (func (result i32))) (func $f (type $f) i32.const 7) (table 1 (ref $f) (ref.func $f)) (func (export "run") (result i32) i32.const 0 call_indirect (type $f)))
