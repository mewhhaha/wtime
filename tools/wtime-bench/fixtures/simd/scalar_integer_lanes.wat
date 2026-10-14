(module
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $a i32) (local $b i32) (local $c i32) (local $d i32)
    (local.set $a (i32.const 2147483647))
    (local.set $b (i32.const -2))
    (local.set $c (i32.const 3))
    (local.set $d (i32.const 4))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $a (i32.xor (i32.mul (i32.add (local.get $a) (i32.const 1)) (i32.const 3)) (i32.const 324508639)))
        (local.set $b (i32.xor (i32.mul (i32.add (local.get $b) (i32.const -3)) (i32.const 5)) (i32.const -610839777)))
        (local.set $c (i32.xor (i32.mul (i32.add (local.get $c) (i32.const 5)) (i32.const 7)) (i32.const 1431655765)))
        (local.set $d (i32.xor (i32.mul (i32.add (local.get $d) (i32.const 6)) (i32.const 9)) (i32.const -1431655766)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i32.add (i32.add (local.get $a) (local.get $b))
             (i32.add (local.get $c) (local.get $d)))))
