;; SPDX-License-Identifier: CC0-1.0
(module
  (type $resume_step (func (param i32) (result i32)))
  (type $cont (cont $resume_step))
  (tag $yield (type $resume_step))
  (tag $failure (param i32))
  (tag $other0 (param i32))
  (tag $other1 (param i32))
  (tag $other2 (param i32))

  ;; Same executing/captured frame shape as captured_try_table_one, with three
  ;; additional alternatives to $failure. The byte-model pair checks that the
  ;; alternatives remain in the retained module, not cloned into each segment.
  (func (export "run") (result i32)
    (block (result i32)
      (try_table
        (catch $failure 0)
        (catch $other0 0)
        (catch $other1 0)
        (catch $other2 0)
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
