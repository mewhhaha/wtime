//! Source-only P042 draft: bounded, instance-local successful call-site answers.
//! Every caller checks the live target before consulting this cache.
use super::*;

const MAX_SITES: usize = 256;
const MAX_WORK: usize = 100_000;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CheckedTarget {
    pub(crate) function: Func,
    pub(crate) canonical: CanonicalTypeId,
    pub(crate) params: usize,
    pub(crate) results: usize,
    pub(crate) host: bool,
}

#[derive(Clone, Copy, Debug)]
struct Site {
    body: usize,
    pc: usize,
    opcode: u16,
    type_index: usize,
    expected: CanonicalTypeId,
    entries: [Option<CheckedTarget>; 2],
    next: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct CallSites {
    store: u64,
    engine: u64,
    source: Arc<[u8]>,
    sites: Vec<Site>,
    work: usize,
    bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    Hit,
    Miss,
    Declined,
}

impl CallSites {
    pub(crate) fn prepare(store: &Store, module: &Module) -> Option<Self> {
        Self::bounded(store, module, MAX_WORK, MAX_BYTES)
    }

    fn bounded(store: &Store, module: &Module, max_work: usize, max_bytes: usize) -> Option<Self> {
        if store.engine.id != module.data.engine {
            return None;
        }
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        let decoded = module.data.inner.decoded();
        // Charge both complete directory traversals before inspecting sites.
        let mut work = decoded.code.len();
        for code in &decoded.code {
            work = work.checked_add(code.instructions.len().checked_mul(2)?)?;
            if work > max_work {
                return None;
            }
        }
        let mut count = 0usize;
        for code in &decoded.code {
            for instruction in &code.instructions {
                if !matches!(instruction.opcode, 0x11 | 0x13 | 0x14 | 0x15) {
                    continue;
                }
                let index = match instruction.immediate {
                    Immediate::Pair(index, _) if matches!(instruction.opcode, 0x11 | 0x13) => index,
                    Immediate::Index(index) if matches!(instruction.opcode, 0x14 | 0x15) => index,
                    _ => return None,
                };
                let ty = decoded.types.get(index as usize)?;
                work = work
                    .checked_add(ty.params.len())?
                    .checked_add(ty.results.len())?;
                count = count.checked_add(1)?;
                if work > max_work || count > MAX_SITES {
                    return None;
                }
            }
        }
        if count == 0 {
            return None;
        }
        let size = |capacity: usize| {
            capacity
                .checked_mul(std::mem::size_of::<Site>())?
                .checked_add(std::mem::size_of::<Self>())
        };
        if size(count)? > max_bytes {
            return None;
        }
        let mut sites = Vec::new();
        sites.try_reserve_exact(count).ok()?;
        let bytes = size(sites.capacity())?;
        if bytes > max_bytes {
            return None;
        }
        for (body, code) in decoded.code.iter().enumerate() {
            for (pc, instruction) in code.instructions.iter().enumerate() {
                if !matches!(instruction.opcode, 0x11 | 0x13 | 0x14 | 0x15) {
                    continue;
                }
                let type_index = match instruction.immediate {
                    Immediate::Pair(index, _) | Immediate::Index(index) => index as usize,
                    _ => return None,
                };
                sites.push(Site {
                    body,
                    pc,
                    opcode: instruction.opcode,
                    type_index,
                    expected: *module.data.canonical.get(type_index)?,
                    entries: [None; 2],
                    next: 0,
                });
            }
        }
        Some(Self {
            store: store.id,
            engine: store.engine.id,
            source: decoded.bytes.clone(),
            sites,
            work,
            bytes,
        })
    }

    fn site(
        &self,
        store: u64,
        module: &Module,
        body: usize,
        pc: usize,
        opcode: u16,
        type_index: usize,
        target: CheckedTarget,
    ) -> Result<Option<usize>> {
        if self.store != store
            || target.function.0.store != store
            || self.engine != module.data.engine
            || target.canonical.engine_id() != self.engine
            || !Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
            || self.work > MAX_WORK
            || self.bytes > MAX_BYTES
            || self.sites.len() > MAX_SITES
        {
            return Err(trap("compact indirect cache identity"));
        }
        let Ok(index) = self
            .sites
            .binary_search_by_key(&(body, pc), |site| (site.body, site.pc))
        else {
            return Ok(None);
        };
        let site = &self.sites[index];
        if site.opcode != opcode
            || site.type_index != type_index
            || module.data.canonical.get(type_index) != Some(&site.expected)
            || site.expected.engine_id() != self.engine
            || site.next >= 2
        {
            return Err(trap("compact indirect cache source signature"));
        }
        Ok(Some(index))
    }

    pub(crate) fn lookup(
        &self,
        store: u64,
        module: &Module,
        body: usize,
        pc: usize,
        opcode: u16,
        type_index: usize,
        target: CheckedTarget,
    ) -> Result<Lookup> {
        let Some(index) = self.site(store, module, body, pc, opcode, type_index, target)? else {
            return Ok(Lookup::Declined);
        };
        Ok(if self.sites[index].entries.contains(&Some(target)) {
            Lookup::Hit
        } else {
            Lookup::Miss
        })
    }

    // Invoke only after the current canonical registry query has succeeded.
    // Failed queries, null refs, stale handles and table misses never publish.
    pub(crate) fn remember(
        &mut self,
        store: u64,
        module: &Module,
        body: usize,
        pc: usize,
        opcode: u16,
        type_index: usize,
        target: CheckedTarget,
    ) -> Result<bool> {
        let Some(index) = self.site(store, module, body, pc, opcode, type_index, target)? else {
            return Ok(false);
        };
        let site = &mut self.sites[index];
        if site.entries.contains(&Some(target)) {
            return Ok(false);
        }
        let position = site.next;
        let evicted = site.entries[position].is_some();
        site.entries[position] = Some(target);
        site.next = (position + 1) % 2;
        Ok(evicted)
    }
}

impl Store {
    // Uses the common decoded indirect/ref source boundary. It does not move
    // operands or enter a callback; all host/guest/root handling stays shared.
    pub(crate) fn compact_indirect_compatible(
        &mut self,
        instance: usize,
        module: &Module,
        body: usize,
        pc: usize,
        opcode: u16,
        type_index: usize,
        function: Func,
    ) -> Result<bool> {
        let live = self.check(function.0, &self.funcs)?;
        let record = &self.funcs[live];
        let target = CheckedTarget {
            function,
            canonical: record.canonical,
            params: record.ty.params.len(),
            results: record.ty.results.len(),
            host: record.host.is_some(),
        };
        let lookup = match self.instances[instance].call_sites.as_ref() {
            Some(cache) => cache.lookup(self.id, module, body, pc, opcode, type_index, target)?,
            None => Lookup::Declined,
        };
        if lookup == Lookup::Hit {
            #[cfg(feature = "execution-reference")]
            {
                self.compact_indirect_hits = self.compact_indirect_hits.saturating_add(1);
            }
            #[cfg(feature = "performance-counters")]
            self.perf.record(perf::Event::CompactIndirectHit, 1);
            return Ok(true);
        }
        if lookup == Lookup::Miss {
            #[cfg(feature = "execution-reference")]
            {
                self.compact_indirect_misses = self.compact_indirect_misses.saturating_add(1);
            }
            #[cfg(feature = "performance-counters")]
            self.perf.record(perf::Event::CompactIndirectMiss, 1);
        }
        let expected = *module
            .data
            .canonical
            .get(type_index)
            .ok_or_else(|| trap("indirect call expected signature"))?;
        let accepted = self
            .engine
            .canonical
            .lock()
            .map_err(|_| err(ErrorKind::Host, "canonical registry poisoned"))?
            .is_subtype(
                target.canonical,
                expected,
                false,
                self.engine.config.limits.max_type_work,
            )?;
        if accepted && lookup == Lookup::Miss {
            let evicted = self.instances[instance]
                .call_sites
                .as_mut()
                .ok_or_else(|| trap("missing compact indirect cache"))?
                .remember(self.id, module, body, pc, opcode, type_index, target)?;
            if evicted {
                #[cfg(feature = "execution-reference")]
                {
                    self.compact_indirect_evictions =
                        self.compact_indirect_evictions.saturating_add(1);
                }
                #[cfg(feature = "performance-counters")]
                self.perf.record(perf::Event::CompactIndirectEviction, 1);
            }
        }
        Ok(accepted)
    }
}

#[cfg(all(test, feature = "execution-reference"))]
mod tests {
    use super::*;
    include!("../../../tests/performance/p042_private.rs");
}
