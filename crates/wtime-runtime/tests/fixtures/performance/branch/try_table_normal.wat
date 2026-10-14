;; SPDX-License-Identifier: CC0-1.0
(module
  (tag $a)
  (tag $b)
  (tag $c)
  ;; Existing scalar-driver interface: run(i32)->i32. A try_table with three
  ;; handlers is entered on every loop iteration but never throws, exercising
  ;; the normal-path catch-metadata setup cost without host imports.
  (func (export "run") (param $n i32) (result i32)
    (local $i i32)
    (local $sum i32)
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
        (try_table
          (catch $a 0)
          (catch $b 0)
          (catch $c 0)
          nop
        )
        (local.set $sum (i32.add (local.get $sum) (local.get $i)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)
      )
    )
    (local.get $sum)
  )
)
