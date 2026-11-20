use wtime_types::{Error, ErrorKind, FeatureSet};
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    base: usize,
    memory64: bool,
    features: FeatureSet,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            base: 0,
            memory64: true,
            features: FeatureSet::all_phase3_core(),
        }
    }
    pub fn with_features(mut self, features: FeatureSet) -> Self {
        self.memory64 = features.memory64;
        self.features = features;
        self
    }
    pub(crate) fn features(&self) -> FeatureSet {
        self.features
    }
    pub(crate) fn feature(&self, enabled: bool, name: &str) -> Result<()> {
        if enabled {
            Ok(())
        } else {
            Err(
                Error::new(ErrorKind::DisabledFeature, format!("{name} disabled"))
                    .at(self.offset()),
            )
        }
    }
    pub fn with_memory64(mut self, enabled: bool) -> Self {
        self.memory64 = enabled;
        self
    }
    pub(crate) fn memory_offset(&mut self) -> Result<u64> {
        self.unsigned(if self.memory64 { 64 } else { 32 })
    }
    pub fn offset(&self) -> usize {
        self.base + self.pos
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
    pub fn error(&self, msg: &str) -> Error {
        Error::new(ErrorKind::Decode, msg).at(self.offset())
    }
    pub fn byte(&mut self) -> Result<u8> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| self.error("unexpected end of input"))?;
        self.pos += 1;
        Ok(b)
    }
    pub fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| self.error("unexpected end of input"))
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| self.error("length overflow"))?;
        let s = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.error("length exceeds input"))?;
        self.pos = end;
        Ok(s)
    }
    pub fn child(&mut self, n: usize) -> Result<Self> {
        let base = self.offset();
        Ok(Self {
            bytes: self.take(n)?,
            pos: 0,
            base,
            memory64: self.memory64,
            features: self.features,
        })
    }
    pub fn finish(&self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(self.error("trailing bytes"))
        }
    }
    #[inline]
    pub fn unsigned(&mut self, bits: u32) -> Result<u64> {
        // One-byte encodings, the common case.
        if (7..=64).contains(&bits)
            && let Some(&b) = self.bytes.get(self.pos)
            && b < 0x80
        {
            self.pos += 1;
            return Ok(b as u64);
        }
        self.unsigned_slow(bits)
    }
    fn unsigned_slow(&mut self, bits: u32) -> Result<u64> {
        if bits == 0 || bits > 64 {
            return Err(self.error("unsupported integer width"));
        }
        let mut v = 0u64;
        for i in 0..bits.div_ceil(7) {
            let b = self.byte()?;
            let payload = (b & 127) as u64;
            let shift = i * 7;
            let available = (bits - shift).min(7);
            if payload >> available != 0 {
                return Err(self.error("unsigned LEB overflow"));
            }
            v |= payload << shift;
            if b & 128 == 0 {
                return Ok(v);
            }
        }
        Err(self.error("unterminated LEB"))
    }
    #[inline]
    pub fn signed(&mut self, bits: u32) -> Result<i64> {
        if (7..=64).contains(&bits)
            && let Some(&b) = self.bytes.get(self.pos)
            && b < 0x80
        {
            self.pos += 1;
            return Ok((((b << 1) as i8) >> 1) as i64);
        }
        self.signed_slow(bits)
    }
    fn signed_slow(&mut self, bits: u32) -> Result<i64> {
        if bits == 0 || bits > 64 {
            return Err(self.error("unsupported integer width"));
        }
        let mut v = 0u64;
        for i in 0..bits.div_ceil(7) {
            let b = self.byte()?;
            let p = (b & 127) as u64;
            let shift = i * 7;
            let available = (bits - shift).min(7);
            if available < 7 {
                let upper = p >> available;
                let sign = (p >> (available - 1)) & 1;
                let expected = if sign == 1 {
                    (1 << (7 - available)) - 1
                } else {
                    0
                };
                if upper != expected {
                    return Err(self.error("signed LEB overflow"));
                }
            }
            v |= p << shift;
            if b & 128 == 0 {
                let used = (shift + 7).min(64);
                if b & 64 != 0 && used < 64 {
                    v |= (!0u64) << used;
                }
                return Ok(v as i64);
            }
        }
        Err(self.error("unterminated LEB"))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(self.unsigned(32)? as u32)
    }
    pub fn name(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        std::str::from_utf8(s)
            .map(str::to_owned)
            .map_err(|_| self.error("invalid UTF-8 name"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t015_padded_and_overflow() {
        assert_eq!(Reader::new(&[0x80, 0x80, 0x80, 0x80, 0]).u32().unwrap(), 0);
        assert!(Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x10]).u32().is_err());
        assert_eq!(
            Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x7f])
                .signed(32)
                .unwrap(),
            -1
        );
        assert!(
            Reader::new(&[0xff, 0xff, 0xff, 0xff, 0x0f])
                .signed(32)
                .is_err()
        );
    }
    #[test]
    fn t016_child_bounds() {
        let mut r = Reader::new(&[1, 2]);
        r.byte().unwrap();
        let mut c = r.child(1).unwrap();
        assert_eq!(c.offset(), 1);
        c.byte().unwrap();
        assert!(c.byte().is_err());
    }
    #[test]
    fn t015_invalid_reader_width() {
        assert!(Reader::new(&[0]).unsigned(65).is_err());
        assert!(Reader::new(&[0]).signed(0).is_err());
    }
}
