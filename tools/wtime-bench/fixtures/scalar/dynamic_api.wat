(module
  (func (export "run") (param $n i32) (result i32)
    (i32.add (local.get $n) (i32.const 1))))
