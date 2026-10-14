#!/usr/bin/env node
// Development-only independent V8 oracle. This adapter accepts no-argument i32
// exports, which is the exact capability profile of differential.py's generator.
const fs = require('node:fs');
const [, , profile, file, name] = process.argv;
(async () => {
  try {
    const module = await WebAssembly.compile(fs.readFileSync(file));
    const instance = await WebAssembly.instantiate(module, {});
    const value = instance.exports[name]();
    if (typeof value !== 'number' || !Number.isInteger(value)) {
      console.log(JSON.stringify({status:'unsupported',message:'oracle adapter only supports i32 result exports'}));
      return;
    }
    console.log(JSON.stringify({status:'returned',values:[{type:'i32',bits:String(value >>> 0)}]}));
  } catch (error) {
    if (error instanceof WebAssembly.RuntimeError) {
      console.log(JSON.stringify({status:'trap',trap:/unreachable/i.test(error.message)?'Unreachable':'Other'}));
    } else if (error instanceof WebAssembly.CompileError) {
      console.log(JSON.stringify({status:'unsupported',message:error.message}));
    } else {
      console.log(JSON.stringify({status:'oracle-error',message:String(error)}));
    }
  }
})();
