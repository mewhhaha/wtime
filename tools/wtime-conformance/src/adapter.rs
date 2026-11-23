use std::collections::HashMap;
use wtime_runtime::*;
pub struct Adapter {
    pub engine: Engine,
    pub legacy_syntax: bool,
    pub features: FeatureSet,
    pub worker_timeout: std::time::Duration,
    pub store: Store,
    pub linker: Linker,
    pub prints: crate::spectest::Prints,
    registrations: HashMap<String, Vec<(String, External)>>,
    current: Option<Instance>,
    named: HashMap<String, Instance>,
    definitions: HashMap<String, Module>,
    last_definition: Option<Module>,
    shared_exports: HashMap<String, Vec<(String, External)>>,
    #[cfg(feature = "performance-counters")]
    pub preparation: PreparationCounts,
}
#[cfg(feature = "performance-counters")]
#[derive(Clone, Copy, Debug, Default)]
pub struct PreparationCounts {
    pub compile_count: usize,
    pub published_artifacts: usize,
    pub accepted: usize,
    pub declined: usize,
    pub attempted_work_known_sum: usize,
    pub unpublished_artifacts: usize,
    pub attempted_work_unknown_artifacts: usize,
    pub artifact_retained_budget_bytes: usize,
    pub peak_artifact_retained_budget_bytes: usize,
    pub overflowed: bool,
}
#[cfg(feature = "performance-counters")]
impl PreparationCounts {
    fn add(&mut self, value: usize) -> bool {
        if let Some(sum) = self.compile_count.checked_add(value) {
            self.compile_count = sum;
            true
        } else {
            self.overflowed = true;
            false
        }
    }
    fn observe(&mut self, module: &Module) {
        self.add(1);
        #[cfg(feature = "compact-executor")]
        {
            match module.compact_preparation_counts() {
                Some((accepted, declined, attempted, retained)) => {
                    self.add_published(accepted, declined, attempted, retained);
                }
                None => {
                    self.add_field(0, 1);
                    self.add_field(1, 1);
                }
            }
        }
        #[cfg(not(feature = "compact-executor"))]
        {
            let _ = module;
            self.add_field(0, 1);
            self.add_field(1, 1);
        }
    }
    fn add_field(&mut self, field: u8, value: usize) {
        let slot = match field {
            0 => &mut self.unpublished_artifacts,
            1 => &mut self.attempted_work_unknown_artifacts,
            2 => &mut self.published_artifacts,
            _ => unreachable!(),
        };
        if let Some(sum) = slot.checked_add(value) {
            *slot = sum;
        } else {
            self.overflowed = true;
        }
    }
    fn add_published(
        &mut self,
        accepted: usize,
        declined: usize,
        attempted: usize,
        retained: usize,
    ) {
        self.add_field(2, 1);
        for (slot, value) in [
            (&mut self.accepted, accepted),
            (&mut self.declined, declined),
            (&mut self.attempted_work_known_sum, attempted),
            (&mut self.artifact_retained_budget_bytes, retained),
        ] {
            if let Some(sum) = slot.checked_add(value) {
                *slot = sum;
            } else {
                self.overflowed = true;
            }
        }
        self.peak_artifact_retained_budget_bytes =
            self.peak_artifact_retained_budget_bytes.max(retained);
    }
}
impl Adapter {
    #[cfg(test)]
    pub fn new(features: FeatureSet) -> Result<Self> {
        Self::with_policy(features, ExecutionPolicy::Decoded)
    }
    #[cfg(test)]
    pub fn with_policy(features: FeatureSet, policy: ExecutionPolicy) -> Result<Self> {
        Self::with_policy_and_threshold(features, policy, None)
    }
    pub fn with_policy_and_threshold(
        features: FeatureSet,
        policy: ExecutionPolicy,
        threshold: Option<u64>,
    ) -> Result<Self> {
        if threshold.is_some_and(|value| value == 0 || policy != ExecutionPolicy::Register) {
            return Err(Error::new(
                ErrorKind::Host,
                "a positive entry threshold requires Register execution",
            ));
        }
        let config = Config {
            features,
            limits: ResourceLimits {
                fuel: Some(10_000_000),
                max_call_depth: 512,
                max_memory_pages: u64::MAX,
                max_memory_bytes: 64 * 1024 * 1024,
                // WTIME_COMPILE_THREADS=n decodes and validates function
                // bodies on n threads even for small modules (n >= 2), to
                // check that results do not depend on it.
                compile_threads: std::env::var("WTIME_COMPILE_THREADS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                ..ResourceLimits::default()
            },
        };
        let engine = match threshold {
            #[cfg(feature = "register-executor")]
            Some(value) => Engine::with_register_entry_threshold(config, value)?,
            #[cfg(not(feature = "register-executor"))]
            Some(_) => {
                return Err(Error::new(
                    ErrorKind::Host,
                    "Register executor is unavailable",
                ));
            }
            None => Engine::with_execution_policy(config, policy)?,
        };
        Self::from_engine(engine, features)
    }
    fn from_engine(engine: Engine, features: FeatureSet) -> Result<Self> {
        let mut store = Store::new(&engine);
        let mut linker = Linker::new(&engine);
        let (prints, exports) =
            crate::spectest::install(&mut store, &mut linker, features.memory64)?;
        Ok(Self {
            store,
            linker,
            engine,
            legacy_syntax: false,
            features,
            worker_timeout: std::time::Duration::from_secs(30),
            prints,
            registrations: HashMap::from([("spectest".into(), exports)]),
            current: None,
            named: HashMap::new(),
            definitions: HashMap::new(),
            last_definition: None,
            shared_exports: HashMap::new(),
            #[cfg(feature = "performance-counters")]
            preparation: PreparationCounts::default(),
        })
    }
    pub fn thread_child(&self, shared: Option<&str>) -> Result<Self> {
        let mut child = Self::from_engine(self.engine.clone(), self.features)?;
        child.worker_timeout = self.worker_timeout;
        if let Some(name) = shared {
            let exports = if let Some(exports) = self.shared_exports.get(name) {
                exports.clone()
            } else {
                self.instance(Some(name))?.exports(&self.store)?
            };
            let mut imported = Vec::new();
            for (n, e) in exports {
                if let External::Memory(m) = e {
                    let token = self.store.share_memory(m)?;
                    let m = child.store.import_shared_memory(&token)?;
                    imported.push((n, External::Memory(m)));
                } else {
                    return Err(Error::new(
                        ErrorKind::Host,
                        "unsupported: shared module non-memory export",
                    ));
                }
            }
            child.shared_exports.insert(name.into(), imported);
        }
        Ok(child)
    }
    pub fn module(&mut self, bytes: &[u8], name: Option<&str>) -> Result<()> {
        let m = self.engine.compile(bytes)?;
        #[cfg(feature = "performance-counters")]
        self.preparation.observe(&m);
        let i = self.linker.instantiate(&mut self.store, &m)?;
        if let Some(n) = name {
            self.named.insert(n.into(), i);
        }
        self.current = Some(i);
        Ok(())
    }
    pub fn definition(&mut self, bytes: &[u8], name: Option<&str>) -> Result<()> {
        let m = self.engine.compile(bytes)?;
        #[cfg(feature = "performance-counters")]
        self.preparation.observe(&m);
        if let Some(name) = name {
            self.definitions.insert(name.into(), m.clone());
        }
        self.last_definition = Some(m);
        Ok(())
    }
    pub fn instantiate_definition(
        &mut self,
        module: Option<&str>,
        name: Option<&str>,
    ) -> Result<()> {
        let m = if let Some(n) = module {
            self.definitions.get(n)
        } else {
            self.last_definition.as_ref()
        }
        .ok_or_else(|| Error::new(ErrorKind::Link, "unknown module definition"))?;
        let i = self.linker.instantiate(&mut self.store, m)?;
        if let Some(n) = name {
            self.named.insert(n.into(), i);
        }
        self.current = Some(i);
        Ok(())
    }
    fn instance(&self, name: Option<&str>) -> Result<&Instance> {
        name.and_then(|n| self.named.get(n))
            .or_else(|| {
                if name.is_none() {
                    self.current.as_ref()
                } else {
                    None
                }
            })
            .ok_or_else(|| Error::new(ErrorKind::Link, "unknown script module"))
    }
    pub fn register(&mut self, name: &str, module: Option<&str>) -> Result<()> {
        let exports = if let Some(exports) = module.and_then(|n| self.shared_exports.get(n)) {
            exports.clone()
        } else {
            self.instance(module)?.exports(&self.store)?
        };
        self.registrations.insert(name.into(), exports);
        let mut linker = Linker::new(&self.engine);
        for (module, exports) in &self.registrations {
            for (name, value) in exports {
                linker.define(module, name, *value)?;
            }
        }
        self.linker = linker;
        Ok(())
    }
    pub fn invoke(
        &mut self,
        module: Option<&str>,
        name: &str,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        let e = self.instance(module)?.export(&self.store, name)?;
        if let External::Func(f) = e {
            match self.store.invoke(f, args)? {
                RunOutcome::Returned(values) => Ok(values),
                RunOutcome::Exception(_) => Err(Error::new(ErrorKind::Host, "guest exception")),
                RunOutcome::Suspended { .. } => {
                    Err(Error::new(ErrorKind::Host, "guest suspension"))
                }
            }
        } else {
            Err(Error::new(ErrorKind::Link, "export is not function"))
        }
    }
    pub fn invoke_outcome(
        &mut self,
        module: Option<&str>,
        name: &str,
        args: &[Value],
    ) -> Result<RunOutcome> {
        if let External::Func(f) = self.instance(module)?.export(&self.store, name)? {
            self.store.invoke(f, args)
        } else {
            Err(Error::new(ErrorKind::Link, "export is not function"))
        }
    }
    pub fn function_ref(&self, module: Option<&str>, index: u32) -> Result<Value> {
        Ok(Value::Ref(RefValue::Func(
            self.instance(module)?.function(&self.store, index)?,
        )))
    }
    pub fn get(&self, module: Option<&str>, name: &str) -> Result<Vec<Value>> {
        if let External::Global(g) = self.instance(module)?.export(&self.store, name)? {
            Ok(vec![self.store.get_global(g)?])
        } else {
            Err(Error::new(ErrorKind::Link, "export is not global"))
        }
    }
}
