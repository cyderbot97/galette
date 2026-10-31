//! Whether the drive keeps the audio it read in a cache, and how much must be
//! read elsewhere on the disc to empty it.
//!
//! Secure mode compares several reads of a sector: if the drive answers a
//! re-read from its cache, the "new" read is the old one and proves nothing.
//! Before each comparison pass, [`rip`](crate::rip) therefore reads another
//! part of the disc, as much as the cache holds. Drives do not say how they
//! cache audio, and some report no cache size or a wrong one, so this is
//! measured, by time: a sector read again at once comes back far faster
//! than from the disc when it is cached. After reading elsewhere, a sector
//! whose re-read is slow again has left the cache.

use std::time::{Duration, Instant};

use crate::Result;
use crate::drive::Drive;
use crate::rip;
use crate::toc::{SECTOR_BYTES, Toc};

/// Largest amount read to empty the cache, in KiB.
pub const MAX_FLUSH_KIB: u32 = 32 * 1024;
/// Sectors probed, in hundredths of the disc: far from the end of the disc,
/// which is what is read to empty the cache, and from one another.
const PROBES: [u32; 3] = [30, 36, 42];

#[derive(Debug, Clone, PartialEq)]
pub struct CacheTest {
    /// The drive answers a sector read again at once from its cache.
    pub caches: bool,
    /// Typical time to read a sector from the disc, and to read it again at once.
    pub disc_read: Duration,
    pub reread: Duration,
    /// Amount to read elsewhere to empty the cache, in KiB (at least
    /// [`rip::MIN_CACHE_KIB`]); `None` if even [`MAX_FLUSH_KIB`] does not.
    pub flush_kib: Option<u32>,
}

/// Measures the drive's cache, then tries emptying it by reading
/// `cache_kib` (as the rip would), then twice as much, and so on.
/// `progress` receives the amount being tried.
pub fn test(drive: &mut Drive, toc: &Toc, cache_kib: u32, progress: &mut dyn FnMut(u32)) -> Result<CacheTest> {
    let disc_end = toc.audio_lead_out();
    let probes = PROBES.map(|p| (disc_end as u64 * p as u64 / 100) as u32);
    let mut buf = vec![0u8; SECTOR_BYTES];
    let mut timed = |drive: &mut Drive, lba: u32| -> Result<Duration> {
        let start = Instant::now();
        drive.read_audio(lba, 1, &mut buf)?;
        Ok(start.elapsed())
    };

    let mut disc = Vec::new();
    let mut reread = Vec::new();
    for &lba in &probes {
        disc.push(timed(drive, lba)?);
        reread.push(timed(drive, lba)?);
    }
    let (disc_read, reread) = (median(disc), median(reread));
    let caches = reread * 3 < disc_read;
    let mut result = CacheTest { caches, disc_read, reread, flush_kib: None };
    let first = cache_kib.max(rip::MIN_CACHE_KIB);
    if !caches {
        result.flush_kib = Some(first);
        return Ok(result);
    }

    // A re-read slower than this comes from the disc.
    let from_disc = Duration::from_secs_f64((disc_read.as_secs_f64() * result.reread.as_secs_f64()).sqrt());
    let mut kib = first;
    while kib <= MAX_FLUSH_KIB {
        progress(kib);
        let sectors = rip::cache_sectors(kib);
        let mut emptied = true;
        for &lba in &probes {
            timed(drive, lba)?;
            rip::flush_cache(drive, lba, disc_end, sectors, None)?;
            emptied &= timed(drive, lba)? > from_disc;
        }
        if emptied {
            result.flush_kib = Some(kib);
            break;
        }
        // A short disc cannot hold a larger flush: reading more is capped.
        if rip::cache_sectors(kib * 2) >= disc_end / 4 {
            break;
        }
        kib *= 2;
    }
    Ok(result)
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};

    use super::*;
    use crate::drive::Transport;

    /// A drive keeping the last `capacity` sectors it read. Reading a sector
    /// alone from the disc takes a few milliseconds; everything else is
    /// instantaneous, to keep the test quick.
    #[derive(Default)]
    struct CachingDrive {
        capacity: usize,
        order: VecDeque<u32>,
        cache: HashSet<u32>,
    }

    impl Transport for CachingDrive {
        fn read(&mut self, cdb: &[u8], buf: &mut [u8], _: Duration) -> Result<usize> {
            let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap());
            let count = u32::from_be_bytes([0, cdb[6], cdb[7], cdb[8]]);
            for sector in lba..lba + count {
                if self.cache.contains(&sector) {
                    continue;
                }
                if count == 1 {
                    std::thread::sleep(Duration::from_millis(4));
                }
                self.cache.insert(sector);
                self.order.push_back(sector);
                if self.order.len() > self.capacity {
                    self.cache.remove(&self.order.pop_front().unwrap());
                }
            }
            Ok(buf.len())
        }
    }

    fn test_drive(capacity: usize, cache_kib: u32) -> CacheTest {
        let toc = Toc::parse_musicbrainz("1 2 200150 150 100150").unwrap();
        let mut drive = Drive::new(Box::new(CachingDrive { capacity, ..Default::default() }));
        test(&mut drive, &toc, cache_kib, &mut |_| {}).unwrap()
    }

    #[test]
    fn finds_how_much_empties_the_cache() {
        // 2 MiB (891 sectors) is the least read: enough for a small cache.
        let small = test_drive(500, 0);
        assert!(small.caches);
        assert_eq!(small.flush_kib, Some(2048));
        // 3.4 MiB needs 4 MiB.
        assert_eq!(test_drive(1500, 0).flush_kib, Some(4096));
        // The size the drive reports is tried first.
        assert_eq!(test_drive(1500, 8192).flush_kib, Some(8192));
    }

    #[test]
    fn a_cache_too_large_is_reported() {
        let huge = test_drive(20_000, 0);
        assert!(huge.caches);
        assert_eq!(huge.flush_kib, None);
    }

    #[test]
    fn a_drive_without_cache() {
        let none = test_drive(0, 0);
        assert!(!none.caches);
        assert_eq!(none.flush_kib, Some(2048));
    }
}
