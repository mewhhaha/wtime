;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module (func (export "run")
  (param $first externref) (param $second externref) (param $condition i32) (result externref)
  local.get $first
  local.get $second
  local.get $condition
  select (result externref)))
