;; SPDX-License-Identifier: CC0-1.0
;; P113 draft fixture. Encoded and validated by the integrator before installation.
(module
  (type $box (struct (field i32)))
  (memory (export "memory") 1)
  (func (export "run") (param $n i32) (result i32)
    (ref.is_null (ref.null $box))
  )
)

