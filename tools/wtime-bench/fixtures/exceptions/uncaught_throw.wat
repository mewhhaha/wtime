(module
  (tag (export "number") (param i32))
  (func (export "run") (param $value i32)
    (local.get $value)
    (throw 0)))
