;; SPDX-License-Identifier: CC0-1.0
(module
 (type $f (func (result i32)))
 (type $resume_f (func (param i32) (result i32)))
 (type $cr (cont $resume_f))
 (tag $yield (result i32))
 (tag $failure (export "failure") (param i32))
 (func $caught (type $f)
  i32.const 4 i32.const 5 i32.add drop
  block (result i32)
   try_table (result i32) (catch $failure 0)
    suspend $yield
   end
  end)
 (func (export "run_caught") (type $f)
  i32.const 1 drop call $caught)
 (func (export "run_plain") (type $f)
  i32.const 6 i32.const 7 i32.add drop suspend $yield)
 (func (export "resume") (param (ref $cr)) (param i32) (result i32)
  local.get 1 local.get 0 resume $cr)
 (func (export "resume_throw") (param (ref $cr)) (param i32) (result i32)
  local.get 1 local.get 0 resume_throw $cr $failure)
 (func (export "resume_throw_caught_here") (param (ref $cr)) (param i32) (result i32)
  i32.const 2 i32.const 3 i32.add drop
  try_table (catch $failure 0)
   local.get 1 local.get 0 resume_throw $cr $failure drop
  end
  i32.const -1))
