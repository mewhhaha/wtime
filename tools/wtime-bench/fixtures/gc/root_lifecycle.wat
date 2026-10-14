(module
  (type $root (struct (field i32)))
  (func (export "make") (param $value i32) (result (ref $root))
    (struct.new $root (local.get $value)))
  (func (export "read") (param $value (ref $root)) (result i32)
    (struct.get $root 0 (local.get $value)))
  (func (export "checksum") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (block $done
      (loop $alloc
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (drop (struct.new $root (local.get $index)))
        (local.set $sum (i32.add (local.get $sum) (local.get $index)))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $alloc)))
    (local.get $sum)))
