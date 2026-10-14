;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module
 (func (export "f32_negative_zero") (result i32) f32.const -0 i32.reinterpret_f32)
 (func (export "f64_negative_zero") (result i64) f64.const -0 i64.reinterpret_f64)
 (func (export "f32_payload") (result i32) f32.const nan:0x412345 i32.reinterpret_f32)
 (func (export "f64_payload") (result i64) f64.const -nan:0x8000000001234 i64.reinterpret_f64))
