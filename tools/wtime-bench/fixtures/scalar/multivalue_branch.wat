(module
  (type $pair (func (param i32 i32) (result i32 i32)))
  (func $choose (type $pair) (param $acc i32) (param $i i32) (result i32 i32)
    (if (result i32 i32) (i32.and (local.get $i) (i32.const 1))
      (then
        (i32.add (local.get $acc) (i32.const 1))
        (i32.add (local.get $i) (i32.const 2)))
      (else
        (i32.mul (local.get $acc) (i32.const 3))
        (i32.sub (local.get $i) (i32.const 1)))))
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $acc i32) (local $discard i32)
    (local.set $acc (i32.const 1))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (call $choose (local.get $acc) (local.get $i))
        (local.set $discard)
        (local.set $acc)
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))
