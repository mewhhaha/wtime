;; SPDX-License-Identifier: CC0-1.0
(module
  (type $resume_step (func (param i32) (result i32)))
  (type $cont (cont $resume_step))
  (tag $yield (type $resume_step))
  (tag $failure (param i32))

  ;; The continuation captures a try_table label. After resumption, the frame
  ;; throws 29 and the captured handler returns that payload through its block.
  (func (export "run") (result i32)
    (block (result i32)
      (try_table (catch $failure 0)
        (i32.const 7)
        (suspend $yield)
        (drop)
        (i32.const 29)
        (throw $failure)
      )
      (i32.const 0)
    )
  )
  (func (export "resume") (param (ref $cont)) (result i32)
    (i32.const 9)
    (local.get 0)
    (resume $cont))
)
