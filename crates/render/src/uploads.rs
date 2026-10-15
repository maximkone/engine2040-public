//! uploads (RA6): generational staging-belt model of the GPU upload discipline.
//! Owner: render staging belt; consumers: RA1 presenter submit path (via the
//! orchestrator's `lib.rs` registration) and the AH2 heal path, which calls
//! [`StagingBelt::invalidate_all`] on resize/surface-loss/device-loss.
//! Units: bytes (`usize`) for sizes, frame counts (`u64`) for generations.
//! Determinism: pure counters with stable Vec order; no clock, no threads.
//! Errors: [`UploadError`] — use-after-retire names id+generation,
//! over-capacity names bytes; nothing is silently dropped.
//! Budget: one Vec entry (~24 B CPU bookkeeping) per live upload; `upload`
//! and `read` scan live entries linearly; `advance_frame` retires in place.

/// In-flight frame depth: an upload issued at generation `g` stays live
/// while `frame - g < IN_FLIGHT_FRAMES` and retires once the belt has
/// advanced `IN_FLIGHT_FRAMES` frames past it.
pub const IN_FLIGHT_FRAMES: u64 = 3;

/// Default staging budget: 64 MiB of live (unretired, uninvalidated) bytes.
pub const DEFAULT_BELT_BYTES: usize = 64 * 1024 * 1024;

/// Generational staging handle. `serial` is unique per upload; `generation`
/// is the belt frame the upload was issued on. Handles never recycle: a
/// retired serial stays retired, so stale reads fail loudly by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StagingId {
    /// Unique upload serial, assigned in issue order from 0.
    pub serial: u64,
    /// Belt frame ([`StagingBelt::frame`]) the upload was issued on.
    pub generation: u64,
}

/// Typed staging-belt failure. Every refusal is loud: the caller either
/// retries after retirement, waits for the heal path, or shrinks the upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadError {
    /// Read of an upload that already retired (aged out by `advance_frame`)
    /// or was dropped by [`StagingBelt::invalidate_all`]. Names the stale
    /// handle so the caller can prove which frame bound it.
    Retired {
        /// Stale upload serial from [`StagingId::serial`].
        serial: u64,
        /// Generation the stale upload was issued on.
        generation: u64,
    },
    /// Upload refused: `bytes` did not fit the remaining belt budget.
    /// Nothing was stored or dropped. Names the refused size and the belt
    /// capacity so the caller can split or evict before retrying.
    OverCapacity {
        /// Refused upload size in bytes.
        bytes: usize,
        /// Belt capacity in bytes at refusal time.
        capacity: usize,
    },
    /// Handle that was never issued by this belt (`serial` at or past the
    /// issue counter). Forged or cross-belt handles fail here, never as
    /// silent reads.
    Unknown {
        /// Unrecognized upload serial.
        serial: u64,
    },
}

impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retired { serial, generation } => write!(
                f,
                "staging belt: upload #{serial} from generation {generation} already retired"
            ),
            Self::OverCapacity { bytes, capacity } => write!(
                f,
                "staging belt: upload of {bytes} bytes refused (capacity {capacity} bytes)"
            ),
            Self::Unknown { serial } => {
                write!(f, "staging belt: unknown upload #{serial}")
            }
        }
    }
}

impl std::error::Error for UploadError {}

/// One live upload reservation: serial, issuing generation, byte size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    serial: u64,
    generation: u64,
    bytes: usize,
}

/// CPU-side model of the GPU staging discipline (no wgpu calls): a ring of
/// `IN_FLIGHT_FRAMES` generations. `upload` binds bytes to the current
/// generation; `advance_frame` retires generations older than every
/// in-flight frame; `invalidate_all` is the AH2 heal hook (resize,
/// surface loss, device loss) that drops all generations at once.
#[derive(Clone, Debug)]
pub struct StagingBelt {
    capacity_bytes: usize,
    live: Vec<Entry>,
    live_bytes: usize,
    high_water_bytes: usize,
    frame: u64,
    next_serial: u64,
}

impl StagingBelt {
    /// New belt holding at most `capacity_bytes` live bytes.
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            live: Vec::new(),
            live_bytes: 0,
            high_water_bytes: 0,
            frame: 0,
            next_serial: 0,
        }
    }

    /// Belt capacity in bytes.
    pub fn capacity_bytes(&self) -> usize {
        self.capacity_bytes
    }

    /// Current generation (frame counter). Binds every new upload.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Live (unretired) byte count.
    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    /// Live (unretired) upload count.
    pub fn live_uploads(&self) -> usize {
        self.live.len()
    }

    /// Peak live-byte count since construction. Never decreases; survives
    /// both `advance_frame` retirement and `invalidate_all` so the heal
    /// path cannot hide the worst case from telemetry.
    pub fn high_water_bytes(&self) -> usize {
        self.high_water_bytes
    }

    /// Reserve `bytes` of staging for the current generation. Refuses with
    /// [`UploadError::OverCapacity`] when the single upload exceeds capacity
    /// or the live total would exceed it; the belt is unchanged on refusal.
    pub fn upload(&mut self, bytes: usize) -> Result<StagingId, UploadError> {
        if bytes > self.capacity_bytes {
            return Err(UploadError::OverCapacity {
                bytes,
                capacity: self.capacity_bytes,
            });
        }
        let total = self.live_bytes.saturating_add(bytes);
        if total > self.capacity_bytes {
            return Err(UploadError::OverCapacity {
                bytes,
                capacity: self.capacity_bytes,
            });
        }
        let id = StagingId {
            serial: self.next_serial,
            generation: self.frame,
        };
        self.next_serial = self.next_serial.saturating_add(1);
        self.live.push(Entry {
            serial: id.serial,
            generation: id.generation,
            bytes,
        });
        self.live_bytes = total;
        if self.live_bytes > self.high_water_bytes {
            self.high_water_bytes = self.live_bytes;
        }
        Ok(id)
    }

    /// Read back the byte size bound to `id`. Live handles succeed; retired
    /// or invalidated handles fail with [`UploadError::Retired`] naming the
    /// stale id+generation; never-issued handles fail with
    /// [`UploadError::Unknown`].
    pub fn read(&self, id: StagingId) -> Result<usize, UploadError> {
        for entry in &self.live {
            if entry.serial == id.serial {
                if entry.generation == id.generation {
                    return Ok(entry.bytes);
                }
                return Err(UploadError::Retired {
                    serial: id.serial,
                    generation: id.generation,
                });
            }
        }
        if id.serial >= self.next_serial {
            return Err(UploadError::Unknown { serial: id.serial });
        }
        Err(UploadError::Retired {
            serial: id.serial,
            generation: id.generation,
        })
    }

    /// True while `read(id)` would succeed.
    pub fn is_live(&self, id: StagingId) -> bool {
        self.read(id).is_ok()
    }

    /// End the current frame: advance one generation and retire every upload
    /// older than all in-flight frames (`frame - generation >=
    /// IN_FLIGHT_FRAMES`). Retired bytes leave the live total; the
    /// high-water mark is kept.
    pub fn advance_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
        let frame = self.frame;
        let mut retired_bytes: usize = 0;
        self.live.retain(|entry| {
            let age = frame.saturating_sub(entry.generation);
            if age >= IN_FLIGHT_FRAMES {
                retired_bytes = retired_bytes.saturating_add(entry.bytes);
                false
            } else {
                true
            }
        });
        self.live_bytes = self.live_bytes.saturating_sub(retired_bytes);
    }

    /// Drop every generation at once. AH2 heal call site: the surface /
    /// device recovery path invokes this on resize, swapchain Outdated /
    /// Lost, and device loss, because in-flight frames may still reference
    /// staging the reconfigured surface no longer owns. Afterwards every
    /// prior handle reads as [`UploadError::Retired`]; the frame counter
    /// and high-water mark are preserved and the belt accepts new uploads
    /// immediately.
    pub fn invalidate_all(&mut self) {
        self.live.clear();
        self.live_bytes = 0;
    }
}

impl Default for StagingBelt {
    fn default() -> Self {
        Self::new(DEFAULT_BELT_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_binds_current_generation_and_reads_back() {
        let mut belt = StagingBelt::new(1024);
        assert_eq!(belt.frame(), 0);
        let id = belt.upload(128).expect("fits");
        assert_eq!((id.serial, id.generation), (0, 0));
        assert_eq!(belt.read(id), Ok(128));
        assert!(belt.is_live(id));
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (128, 1));
    }

    #[test]
    fn read_stays_live_inside_window_then_retires_loudly() {
        let mut belt = StagingBelt::new(1024);
        let id = belt.upload(64).expect("fits");
        belt.advance_frame();
        assert_eq!(belt.read(id), Ok(64));
        belt.advance_frame();
        assert_eq!(belt.read(id), Ok(64));
        belt.advance_frame();
        let err = belt.read(id).expect_err("gen 0 retired at frame 3");
        assert_eq!(
            err,
            UploadError::Retired {
                serial: id.serial,
                generation: id.generation,
            }
        );
        let text = err.to_string();
        assert!(text.contains("#0"), "names id: {text}");
        assert!(text.contains("generation 0"), "names generation: {text}");
        assert!(!belt.is_live(id));
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (0, 0));
    }

    #[test]
    fn ring_retires_old_generation_while_young_stays_live() {
        let mut belt = StagingBelt::new(1024);
        let old = belt.upload(100).expect("gen 0");
        belt.advance_frame();
        let young = belt.upload(200).expect("gen 1");
        belt.advance_frame();
        belt.advance_frame();
        assert_eq!(belt.frame(), 3);
        assert_eq!(
            belt.read(old),
            Err(UploadError::Retired {
                serial: old.serial,
                generation: old.generation,
            })
        );
        assert_eq!(belt.read(young), Ok(200));
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (200, 1));
    }

    #[test]
    fn single_upload_over_capacity_refused_and_stores_nothing() {
        let mut belt = StagingBelt::new(256);
        let err = belt.upload(257).expect_err("single upload over cap");
        assert_eq!(
            err,
            UploadError::OverCapacity {
                bytes: 257,
                capacity: 256,
            }
        );
        let text = err.to_string();
        assert!(text.contains("257"), "names bytes: {text}");
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (0, 0));
    }

    #[test]
    fn cumulative_overflow_refused_without_partial_store() {
        let mut belt = StagingBelt::new(256);
        belt.upload(200).expect("fits");
        let err = belt.upload(100).expect_err("live 200 + 100 over 256");
        assert_eq!(
            err,
            UploadError::OverCapacity {
                bytes: 100,
                capacity: 256,
            }
        );
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (200, 1));
        belt.upload(56).expect("exactly fills");
        assert_eq!(belt.live_bytes(), 256);
    }

    #[test]
    fn high_water_tracks_peak_across_retirement() {
        let mut belt = StagingBelt::new(1024);
        assert_eq!(belt.high_water_bytes(), 0);
        belt.upload(300).expect("fits");
        belt.upload(100).expect("fits");
        assert_eq!(belt.high_water_bytes(), 400);
        for _ in 0..IN_FLIGHT_FRAMES {
            belt.advance_frame();
        }
        assert_eq!((belt.live_bytes(), belt.high_water_bytes()), (0, 400));
        belt.upload(50).expect("fits after retire");
        assert_eq!(belt.high_water_bytes(), 400);
        assert_eq!(belt.live_bytes(), 50);
    }

    #[test]
    fn unknown_handle_refused_not_read_as_retired() {
        let belt = StagingBelt::new(1024);
        let forged = StagingId {
            serial: 7,
            generation: 0,
        };
        assert_eq!(belt.read(forged), Err(UploadError::Unknown { serial: 7 }));
    }

    #[test]
    fn torture_resize_loss_mid_flight_all_reads_fail_then_belt_reused() {
        // Resize/device-loss drill: uploads spread over two frames with one
        // frame still in flight, then the AH2 heal path fires
        // `invalidate_all` mid-flight. No stale handle may read afterwards.
        let mut belt = StagingBelt::new(4096);
        let frame0: Vec<StagingId> = (0..4)
            .map(|_| belt.upload(256).expect("frame 0 fits"))
            .collect();
        belt.advance_frame();
        let frame1: Vec<StagingId> = (0..4)
            .map(|_| belt.upload(256).expect("frame 1 fits"))
            .collect();
        assert_eq!(belt.live_bytes(), 2048);
        for id in frame0.iter().chain(frame1.iter()) {
            assert!(belt.is_live(*id), "live before heal: {id:?}");
        }
        belt.invalidate_all();
        assert_eq!((belt.live_bytes(), belt.live_uploads()), (0, 0));
        for id in frame0.iter().chain(frame1.iter()) {
            let err = belt.read(*id).expect_err("healed handle must refuse");
            assert_eq!(
                err,
                UploadError::Retired {
                    serial: id.serial,
                    generation: id.generation,
                }
            );
        }
        assert_eq!(belt.high_water_bytes(), 2048);
        let fresh = belt.upload(512).expect("belt reusable right after heal");
        assert_eq!(belt.read(fresh), Ok(512));
    }

    #[test]
    fn zero_byte_upload_binds_but_consumes_no_budget() {
        let mut belt = StagingBelt::new(64);
        let id = belt.upload(0).expect("zero-byte binds");
        assert_eq!(belt.read(id), Ok(0));
        assert_eq!(belt.live_bytes(), 0);
        assert_eq!(belt.high_water_bytes(), 0);
    }
}
