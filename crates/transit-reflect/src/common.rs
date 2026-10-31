use xxhash_rust::const_xxh3;

const fn slice_copy(src: &[u8], dst: &mut [u8], loc: usize) {
    let mut i = 0;
    while i < src.len() {
        dst[loc + i] = src[i];
        i += 1;
    }
}

pub(crate) struct Hasher {
    it: [u8; 16],
}

impl Hasher {
    pub const fn new(initial: u8) -> Self {
        let mut it = [0u8; 16];
        it[0] = initial;

        Self { it }
    }

    pub const fn next(&mut self, stuff: &[u8]) {
        let hash = const_xxh3::xxh3_64(stuff);
        slice_copy(&hash.to_le_bytes(), &mut self.it, 8);

        let full_hash = const_xxh3::xxh3_64(&self.it);
        slice_copy(&full_hash.to_le_bytes(), &mut self.it, 0);
    }

    #[allow(unused)]
    pub const fn combine(&mut self, other: Hasher) {
        self.next(&other.finish().to_le_bytes())
    }

    pub const fn finish(self) -> u64 {
        const_xxh3::xxh3_64(&self.it)
    }
}
