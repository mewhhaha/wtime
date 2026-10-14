(module
  (type $packed (array (mut i8)))
  (func (export "run") (param $count i32) (result i32)
    (local $items (ref null $packed))
    (local $index i32)
    (local $sum i32)
    (local.set $items (array.new $packed (i32.const -3) (local.get $count)))
    (block $done
      (loop $scan
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $sum
          (i32.add (local.get $sum)
            (array.get_s $packed (local.get $items) (local.get $index))))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $scan)))
    (local.get $sum)))
