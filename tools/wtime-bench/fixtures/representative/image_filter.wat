;; Original benchmark fixture, authored for wtime; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 1 1)
  (func (export "prepare") (param $width i32)
    (local $i i32) (local $pixel i32)
    (block $input_done
      (loop $make_input
        (br_if $input_done (i32.ge_u (local.get $i) (local.get $width)))
        (local.set $pixel
          (i32.and
            (i32.add (i32.mul (local.get $i) (i32.const 37)) (i32.const 11))
            (i32.const 255)))
        (i32.store8 (local.get $i) (local.get $pixel))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $make_input))))
  (func (export "run") (param $width i32) (result i32)
    (local $i i32) (local $left i32) (local $right i32)
    (local $pixel i32) (local $sum i32)
    (local.set $i (i32.const 0))
    (block $filter_done
      (loop $filter
        (br_if $filter_done (i32.ge_u (local.get $i) (local.get $width)))
        (local.set $left
          (if (result i32) (i32.eqz (local.get $i))
            (then (i32.const 0))
            (else (i32.sub (local.get $i) (i32.const 1)))))
        (local.set $right
          (if (result i32) (i32.ge_u (i32.add (local.get $i) (i32.const 1)) (local.get $width))
            (then (i32.sub (local.get $width) (i32.const 1)))
            (else (i32.add (local.get $i) (i32.const 1)))))
        (local.set $pixel
          (i32.div_u
            (i32.add
              (i32.add (i32.load8_u (local.get $left)) (i32.load8_u (local.get $i)))
              (i32.load8_u (local.get $right)))
            (i32.const 3)))
        (i32.store8 (i32.add (local.get $i) (i32.const 4096)) (local.get $pixel))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $filter)))
    (local.set $i (i32.const 0))
    (block $sum_done
      (loop $sum_output
        (br_if $sum_done (i32.ge_u (local.get $i) (local.get $width)))
        (local.set $sum
          (i32.add (local.get $sum)
                   (i32.load8_u (i32.add (local.get $i) (i32.const 4096)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $sum_output)))
    (local.get $sum)))
