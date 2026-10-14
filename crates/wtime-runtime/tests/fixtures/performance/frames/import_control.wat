;; SPDX-License-Identifier: CC0-1.0
(module
  (import "env" "bump" (func $bump (param i32) (result i32)))
  ;; This direct guest-to-import call must continue through the host boundary.
  (func (export "run") (param i32) (result i32)
    (local.get 0)
    (call $bump)))
