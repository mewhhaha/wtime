//! One-to-one source maps for admitted compact metadata.
//! Dynamic handlers and reference branches decline before publication.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span { first: usize, len: usize }
#[derive(Debug)]
pub(super) struct SourceMap {
    engine: u64,
    source: Arc<[u8]>,
    body: usize,
    source_to_compact: Vec<usize>, // includes terminal n
    compact_to_source: Vec<usize>, // includes terminal n
    successors: Vec<Span>,
    targets: Vec<usize>,
    pub(super) retained_bytes: usize,
    work: usize,
}
#[derive(Clone, Copy)]
struct Label { start: usize, end: usize, loop_: bool }
fn branch(labels: &[Label], depth: u32, terminal: usize) -> Option<usize> {
    let depth = usize::try_from(depth).ok()?;
    if depth == labels.len() { return Some(terminal); }
    let position = labels.len().checked_sub(depth.checked_add(1)?)?;
    let label = labels.get(position)?;
    if label.loop_ { Some(label.start) } else { label.end.checked_add(1) }
}
impl SourceMap {
    #[cfg(test)]
    pub(super) fn p045_corrupt(&mut self, case: usize) {
        match case {
            0 => self.source_to_compact[0] = 1,
            1 => self.targets[0] = usize::MAX,
            2 => self.successors[0].first = usize::MAX,
            3 => self.compact_to_source.clear(),
            _ => unreachable!(),
        }
    }
    pub(super) fn bounded(function: &CompactFunction, module: &Module, max_work: usize, max_bytes: usize)
        -> std::result::Result<Self, Declined>
    {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        if function.work > max_work || function.retained_bytes > max_bytes { return Err(Declined::Budget); }
        if !function.verify(module) { return Err(Declined::Verification); }
        let code = module.data.inner.decoded().code.get(function.body).ok_or(Declined::Verification)?;
        let terminal = code.instructions.len();
        let entries = terminal.checked_add(1).ok_or(Declined::Budget)?;
        let mut work = terminal.checked_mul(8).and_then(|n| n.checked_add(function.work)).ok_or(Declined::Budget)?;
        if function.work.checked_add(work).ok_or(Declined::Budget)? > max_work { return Err(Declined::Budget); }
        let mut edges = 0usize;
        for instruction in &code.instructions {
            // Dynamic handlers/captured cursors require later admitted-origin
            // certificates. Decline a whole function, never guess an edge.
            if matches!(instruction.opcode, 6 | 7 | 8 | 9 | 0xa | 0x18 | 0x19 | 0x1f | 0xfb18 | 0xfb19)
                || matches!(instruction.opcode, 0xe0..=0xe6)
                || matches!(instruction.opcode, 0xfb25 | 0xfb26)
            { return Err(Declined::Unsupported); }
            let count = match (&instruction.immediate, instruction.opcode) {
                (Immediate::BrTable(targets, _), 0xe) => targets.len().checked_add(1).ok_or(Declined::Budget)?,
                (_, 4 | 0xd) => 2,
                (_, 0) => 0,
                _ => 1,
            };
            edges = edges.checked_add(count).ok_or(Declined::Budget)?;
            if function.work.checked_add(work).ok_or(Declined::Budget)?.checked_add(edges.checked_mul(3).ok_or(Declined::Budget)?).ok_or(Declined::Budget)? > max_work { return Err(Declined::Budget); }
        }
        work = work.checked_add(edges.checked_mul(3).ok_or(Declined::Budget)?).ok_or(Declined::Budget)?;
        if function.work.checked_add(work).ok_or(Declined::Budget)? > max_work { return Err(Declined::Budget); }
        // Include transient labels in preparation peak, retained arrays in
        // published-byte count. Actual capacities are checked after reserving.
        let planned = std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<usize>(entries.checked_mul(2).ok_or(Declined::Budget)?).ok_or(Declined::Budget)?)
            .and_then(|n| n.checked_add(vector_bytes::<Span>(terminal)?))
            .and_then(|n| n.checked_add(vector_bytes::<usize>(edges)?))
            .and_then(|n| n.checked_add(vector_bytes::<Label>(terminal)?)).ok_or(Declined::Budget)?;
        if function.retained_bytes.checked_add(planned).ok_or(Declined::Budget)? > max_bytes { return Err(Declined::Budget); }
        let mut map = Self { engine: function.engine, source: function.source.clone(), body: function.body, source_to_compact: Vec::new(), compact_to_source: Vec::new(),
            successors: Vec::new(), targets: Vec::new(), retained_bytes: 0, work };
        let mut labels = Vec::new();
        map.source_to_compact.try_reserve_exact(entries).map_err(|_| Declined::Allocation)?;
        map.compact_to_source.try_reserve_exact(entries).map_err(|_| Declined::Allocation)?;
        map.successors.try_reserve_exact(terminal).map_err(|_| Declined::Allocation)?;
        map.targets.try_reserve_exact(edges).map_err(|_| Declined::Allocation)?;
        labels.try_reserve_exact(terminal).map_err(|_| Declined::Allocation)?;
        let retained = map.bytes().ok_or(Declined::Budget)?;
        let peak = function.retained_bytes.checked_add(retained).ok_or(Declined::Budget)?.checked_add(vector_bytes::<Label>(labels.capacity()).ok_or(Declined::Budget)?).ok_or(Declined::Budget)?;
        if peak > max_bytes { return Err(Declined::Budget); }
        map.source_to_compact.extend(0..entries);
        map.compact_to_source.extend(0..entries);
        for (pc, instruction) in code.instructions.iter().enumerate() {
            let first = map.targets.len();
            let next = pc.checked_add(1).ok_or(Declined::Budget)?;
            match instruction.opcode {
                0 => {},
                2..=4 => {
                    let (end, other) = module.data.control.get(function.body)
                        .and_then(|control| control.get(pc)).copied().flatten().ok_or(Declined::Verification)?;
                    if end >= terminal || end < pc || other.is_some_and(|other| other <= pc || other >= end) {
                        return Err(Declined::Verification);
                    }
                    map.targets.push(next);
                    if instruction.opcode == 4 { map.targets.push(match other { Some(other) => other.checked_add(1).ok_or(Declined::Budget)?, None => end }); }
                    labels.push(Label { start: next, end, loop_: instruction.opcode == 3 });
                },
                5 => map.targets.push(labels.last().ok_or(Declined::Verification)?.end),
                0xb => { labels.pop(); map.targets.push(next); },
                0xc | 0xd => {
                    let Immediate::Index(depth) = instruction.immediate else { return Err(Declined::Verification); };
                    if instruction.opcode == 0xd { map.targets.push(next); }
                    map.targets.push(branch(&labels, depth, terminal).ok_or(Declined::Verification)?);
                },
                0xe => {
                    let Immediate::BrTable(targets, default) = &instruction.immediate else { return Err(Declined::Verification); };
                    for depth in targets.iter().chain(std::iter::once(default)) {
                        map.targets.push(branch(&labels, *depth, terminal).ok_or(Declined::Verification)?);
                    }
                },
                0xf | 0x12 | 0x13 | 0x15 => map.targets.push(terminal),
                // Reference branches need typed label expansion; explicit
                // decline until modeled rather than sequential misattribution.
                0xd5 | 0xd6 => return Err(Declined::Unsupported),
                _ => map.targets.push(next),
            }
            if map.targets[first..].iter().any(|target| *target > terminal) { return Err(Declined::Verification); }
            map.successors.push(Span { first, len: map.targets.len() - first });
        }
        if map.targets.len() != edges || !labels.is_empty() { return Err(Declined::Verification); }
        map.retained_bytes = map.bytes().ok_or(Declined::Budget)?;
        Ok(map)
    }
    fn belongs_to(&self, function: &CompactFunction, module: &Module) -> bool {
        self.engine == function.engine && self.engine == module.data.engine
            && self.body == function.body
            && Arc::ptr_eq(&self.source, &function.source)
            && Arc::ptr_eq(&self.source, &module.data.inner.decoded().bytes)
    }
    pub(super) fn verify(&self, function: &CompactFunction, module: &Module, max_work: usize, max_bytes: usize) -> bool {
        let max_work = max_work.min(MAX_WORK);
        let max_bytes = max_bytes.min(MAX_BYTES);
        if !self.belongs_to(function, module) || self.bytes() != Some(self.retained_bytes)
            || self.retained_bytes > max_bytes || self.work > max_work { return false; }
        // Reconstruction is verification-time scratch. Bound aggregate peak
        // and total comparison work, not just the new map in isolation.
        let Some(remaining_bytes) = max_bytes.checked_sub(self.retained_bytes) else { return false; };
        let Some(compare_work) = self.source_to_compact.len().checked_add(self.compact_to_source.len())
            .and_then(|n| n.checked_add(self.successors.len()))
            .and_then(|n| n.checked_add(self.targets.len())) else { return false; };
        let Some(remaining_work) = max_work.checked_sub(compare_work) else { return false; };
        let Ok(expected) = Self::bounded(function, module, remaining_work, remaining_bytes) else { return false; };
        self.work == expected.work
            && self.source_to_compact == expected.source_to_compact
            && self.compact_to_source == expected.compact_to_source
            && self.successors == expected.successors && self.targets == expected.targets
    }
    fn bytes(&self) -> Option<usize> {
        std::mem::size_of::<Self>()
            .checked_add(vector_bytes::<usize>(self.source_to_compact.capacity())?)?
            .checked_add(vector_bytes::<usize>(self.compact_to_source.capacity())?)?
            .checked_add(vector_bytes::<Span>(self.successors.capacity())?)?
            .checked_add(vector_bytes::<usize>(self.targets.capacity())?)
    }
    fn events(&self, compact: usize) -> Option<std::ops::Range<usize>> {
        // Terminal has no source event; one-to-one is the only admitted shape.
        if compact >= self.successors.len() { return None; }
        Some(compact..compact.checked_add(1)?)
    }
    fn next_source(&self, compact: usize) -> Option<usize> { self.compact_to_source.get(compact).copied() }
    fn destination(&self, source: usize) -> Option<usize> { self.source_to_compact.get(source).copied() }
    pub(super) fn targets(&self, pc: usize) -> Option<&[usize]> {
        let span = self.successors.get(pc)?;
        self.targets.get(span.first..span.first.checked_add(span.len)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("../../../tests/performance/p037_private.rs");
}
