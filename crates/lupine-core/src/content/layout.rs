use std::ops::Range;

use crate::crypto::{TAG_LEN, padme};
use crate::error::{Error, Result};

/// About 6,000 blobs for 50 GB, and one chunk in memory per transfer slot.
pub const DEFAULT_CHUNK_SIZE: u32 = 8 << 20;
pub const MIN_CHUNK_SIZE: u32 = SEGMENT_SIZE as u32;
pub const MAX_CHUNK_SIZE: u32 = 64 << 20;
/// Unit of authenticated decryption, so a read fetches one segment, not a whole chunk.
pub const SEGMENT_SIZE: usize = 64 << 10;
const MIN_PADDED_LEN: u64 = 4096;

/// Fixed-size chunking: chunk `i` covers bytes `[i * chunk_size, (i + 1) * chunk_size)`.
/// Each chunk is encrypted as a run of segments, each followed by its tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkLayout {
    size: u64,
    chunk_size: u32,
}

impl ChunkLayout {
    pub fn new(size: u64, chunk_size: u32) -> Result<Self> {
        if !chunk_size.is_power_of_two() || !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size)
        {
            return Err(Error::InvalidChunkSize(chunk_size));
        }
        Ok(Self { size, chunk_size })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn chunk_size(&self) -> u32 {
        self.chunk_size
    }

    pub fn chunk_count(&self) -> u64 {
        self.size.div_ceil(self.step())
    }

    pub fn chunk_range(&self, index: u64) -> Range<u64> {
        let start = index * self.step();
        start..(start + self.step()).min(self.size)
    }

    pub fn chunk_len(&self, index: u64) -> usize {
        let range = self.chunk_range(index);
        (range.end - range.start) as usize
    }

    /// Plaintext length inside the blob. Only the last chunk is padded, so
    /// blob sizes reveal little more than the size bucket of the file.
    pub fn padded_len(&self, index: u64) -> usize {
        let len = self.chunk_len(index) as u64;
        if index + 1 < self.chunk_count() {
            return len as usize;
        }
        padme(len)
            .max(MIN_PADDED_LEN)
            .min(u64::from(self.chunk_size)) as usize
    }

    pub fn segment_count(&self, index: u64) -> usize {
        self.padded_len(index).div_ceil(SEGMENT_SIZE)
    }

    pub fn blob_len(&self, index: u64) -> usize {
        self.padded_len(index) + self.segment_count(index) * TAG_LEN
    }

    /// Indices of the chunks holding at least one byte of `range`.
    pub fn chunks_covering(&self, range: Range<u64>) -> Range<u64> {
        let range = self.clamp(range);
        if range.is_empty() {
            return 0..0;
        }
        range.start / self.step()..range.end.div_ceil(self.step())
    }

    /// Segments of chunk `index` holding at least one byte of `range`.
    pub fn segments_covering(&self, index: u64, range: Range<u64>) -> Range<usize> {
        let chunk = self.chunk_range(index);
        let start = range.start.max(chunk.start);
        let end = range.end.min(chunk.end);
        if start >= end {
            return 0..0;
        }
        let segment = SEGMENT_SIZE as u64;
        ((start - chunk.start) / segment) as usize..(end - chunk.start).div_ceil(segment) as usize
    }

    /// Where a run of segments sits inside the blob of chunk `index`.
    pub fn segment_blob_range(&self, index: u64, segments: Range<usize>) -> Range<u64> {
        let sealed = (SEGMENT_SIZE + TAG_LEN) as u64;
        let end = (segments.end as u64 * sealed).min(self.blob_len(index) as u64);
        segments.start as u64 * sealed..end
    }

    /// `range` clamped to the file and widened to segment boundaries: what a
    /// download writes.
    pub fn segment_aligned(&self, range: Range<u64>) -> Range<u64> {
        let range = self.clamp(range);
        if range.is_empty() {
            return 0..0;
        }
        let segment = SEGMENT_SIZE as u64;
        range.start / segment * segment..(range.end.div_ceil(segment) * segment).min(self.size)
    }

    fn clamp(&self, range: Range<u64>) -> Range<u64> {
        range.start..range.end.min(self.size)
    }

    fn step(&self) -> u64 {
        u64::from(self.chunk_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CS: u32 = MIN_CHUNK_SIZE;
    const STEP: u64 = CS as u64;
    const SEG: u64 = SEGMENT_SIZE as u64;

    #[test]
    fn rejects_bad_chunk_sizes() {
        for bad in [
            0,
            1000,
            MIN_CHUNK_SIZE / 2,
            MAX_CHUNK_SIZE * 2,
            MIN_CHUNK_SIZE + 1,
        ] {
            assert!(ChunkLayout::new(10, bad).is_err(), "{bad}");
        }
        assert!(ChunkLayout::new(10, DEFAULT_CHUNK_SIZE).is_ok());
    }

    #[test]
    fn counts_and_ranges() {
        let cases = [
            (0, 0),
            (1, 1),
            (STEP - 1, 1),
            (STEP, 1),
            (STEP + 1, 2),
            (3 * STEP + 7, 4),
        ];
        for (size, count) in cases {
            let layout = ChunkLayout::new(size, CS).unwrap();
            assert_eq!(layout.chunk_count(), count, "size {size}");
            let total: u64 = (0..count).map(|i| layout.chunk_len(i) as u64).sum();
            assert_eq!(total, size);
        }
        let layout = ChunkLayout::new(3 * STEP + 7, CS).unwrap();
        assert_eq!(layout.chunk_range(3), 3 * STEP..3 * STEP + 7);
    }

    #[test]
    fn only_the_last_chunk_is_padded() {
        let layout = ChunkLayout::new(2 * STEP + 1000, CS).unwrap();
        assert_eq!(layout.padded_len(0), CS as usize);
        assert_eq!(layout.padded_len(1), CS as usize);
        assert_eq!(layout.padded_len(2), 4096);

        let exact = ChunkLayout::new(2 * STEP, CS).unwrap();
        assert_eq!(exact.padded_len(1), CS as usize);

        let odd = ChunkLayout::new(STEP - 1, CS).unwrap();
        assert_eq!(odd.padded_len(0), CS as usize);
    }

    #[test]
    fn blobs_hold_one_tag_per_segment() {
        let layout =
            ChunkLayout::new(u64::from(DEFAULT_CHUNK_SIZE) + 1000, DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(layout.segment_count(0), 128);
        assert_eq!(layout.blob_len(0), (8 << 20) + 128 * 16);
        assert_eq!(layout.segment_count(1), 1);
        assert_eq!(layout.blob_len(1), 4096 + 16);
    }

    #[test]
    fn covering_ranges() {
        let layout = ChunkLayout::new(3 * STEP + 7, CS).unwrap();
        assert_eq!(layout.chunks_covering(0..1), 0..1);
        assert_eq!(layout.chunks_covering(STEP - 1..STEP + 1), 0..2);
        assert_eq!(layout.chunks_covering(STEP..2 * STEP), 1..2);
        assert_eq!(layout.chunks_covering(3 * STEP..u64::MAX), 3..4);
        assert_eq!(layout.chunks_covering(5..5), 0..0);
        assert_eq!(layout.chunks_covering(10 * STEP..11 * STEP), 0..0);
    }

    #[test]
    fn segment_ranges() {
        let cs = 1 << 20;
        let size = 2 * cs as u64 + 100;
        let layout = ChunkLayout::new(size, cs).unwrap();
        assert_eq!(layout.segments_covering(0, 0..1), 0..1);
        assert_eq!(layout.segments_covering(0, SEG - 1..SEG + 1), 0..2);
        assert_eq!(layout.segments_covering(1, 0..u64::MAX), 0..16);
        assert_eq!(layout.segments_covering(1, 0..cs as u64), 0..0);
        assert_eq!(layout.segments_covering(2, 2 * cs as u64..size), 0..1);

        let sealed = SEG + 16;
        assert_eq!(layout.segment_blob_range(0, 1..3), sealed..3 * sealed);
        assert_eq!(layout.segment_blob_range(2, 0..1), 0..4096 + 16);

        assert_eq!(layout.segment_aligned(SEG + 5..SEG + 6), SEG..2 * SEG);
        assert_eq!(layout.segment_aligned(size - 1..u64::MAX), size - 100..size);
        assert_eq!(layout.segment_aligned(size..size + 10), 0..0);
    }
}
