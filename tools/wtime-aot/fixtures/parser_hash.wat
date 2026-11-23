;; Original benchmark fixture, authored for wtime; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 1 1)
  (func (export "prepare") (param $n i32)
    (local $i i32) (local $byte i32)
    (local.set $i (i32.const 0))
    (block $input_done
      (loop $make_input
        (br_if $input_done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $byte
          (i32.add
            (i32.rem_u (i32.add (i32.mul (local.get $i) (i32.const 17)) (i32.const 3))
                       (i32.const 10))
            (i32.const 48)))
        (i32.store8 (local.get $i) (local.get $byte))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $make_input))))
  (func (export "run") (param $n i32) (result i32)
    (local $i i32) (local $byte i32) (local $hash i32)
    (local.set $i (i32.const 0))
    (local.set $hash (i32.const -2128831035))
    (block $parse_done
      (loop $parse
        (br_if $parse_done (i32.ge_u (local.get $i) (local.get $n)))
        (local.set $byte (i32.load8_u (local.get $i)))
        (local.set $byte (i32.sub (local.get $byte) (i32.const 48)))
        (i32.store8 (i32.add (i32.const 4096) (local.get $i)) (local.get $byte))
        (local.set $hash
          (i32.mul (i32.xor (local.get $hash) (local.get $byte)) (i32.const 16777619)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $parse)))
    (local.get $hash)))
