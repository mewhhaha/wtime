;; SPDX-License-Identifier: CC0-1.0
(module
  (tag $failure (param i32))
  (global $path (export "path") (mut i32) (i32.const 0))
  (func $throw_failure (param $value i32)
    (local.get $value)
    (throw $failure))

  ;; Normal path returns 1105. The inner catch yields 17, followed by 100
  ;; inside the outer try and 1000 outside it. An outer-first catch would
  ;; instead return 1017, making handler order observable.
  (func (export "run") (param $mode i32) (result i32)
    (global.set $path (i32.const 0))
    (block $outer (result i32)
      (try_table (result i32) (catch $failure 0)
        (block $inner (result i32)
          (try_table (result i32) (catch $failure 0)
            (if (local.get $mode)
              (then (call $throw_failure (i32.const 17))))
            (i32.const 5)))
        (i32.const 100)
        (i32.add))
      (i32.const 1000)
      (i32.add))
  )
)
