(module
  (rec
    (type $base (sub (struct (field i32))))
    (type $child (sub $base (struct (field i32) (field i64)))))
  (func (export "run") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (local $value (ref null $base))
    (block $done
      (loop $casts
        (br_if $done (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $value
          (struct.new $child (local.get $index) (i64.extend_i32_u (local.get $index))))
        (local.set $sum
          (i32.add (local.get $sum)
            (ref.test (ref $base) (local.get $value))))
        (local.set $sum
          (i32.add (local.get $sum)
            (ref.test (ref (exact $base)) (local.get $value))))
        (local.set $sum
          (i32.add (local.get $sum)
            (ref.test (ref $child) (local.get $value))))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $casts)))
    (local.get $sum)))
