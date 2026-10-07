//! Copies captured frames from the GPU into memory without stalling rendering: the
//! capture texture is copied into one of a few staging buffers, which are mapped
//! asynchronously and read a frame or two later.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::encode::Frame;
use super::{padded_bytes_per_row, unpad};
use crate::gpu::RenderTarget;
use crate::passes::CAPTURE_FORMAT;

/// Staging buffers in the ring.
pub const SLOTS: usize = 3;

/// Where one staging buffer is in its round trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Free,
    /// A copy of frame `pts` is recorded; mapping starts after the submit.
    Copied(i64),
    /// Mapping of frame `pts` has been requested.
    Mapping(i64),
}

/// The slots, used strictly in rotation so frames come back in order.
#[derive(Clone, Debug)]
pub struct Ring {
    states: [SlotState; SLOTS],
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            states: [SlotState::Free; SLOTS],
            next: 0,
        }
    }
}

impl Ring {
    /// Claims the next slot in rotation for frame `pts`, or `None` while it's still busy.
    pub fn claim(&mut self, pts: i64) -> Option<usize> {
        let i = self.next;
        if self.states[i] != SlotState::Free {
            return None;
        }
        self.states[i] = SlotState::Copied(pts);
        self.next = (i + 1) % SLOTS;
        Some(i)
    }

    /// Marks every copied slot as mapping and returns them.
    pub fn start_mapping(&mut self) -> Vec<usize> {
        let mut started = Vec::new();
        for (i, state) in self.states.iter_mut().enumerate() {
            if let SlotState::Copied(pts) = *state {
                *state = SlotState::Mapping(pts);
                started.push(i);
            }
        }
        started
    }

    /// The mapping slot holding the oldest frame, with its timestamp.
    pub fn oldest_mapping(&self) -> Option<(usize, i64)> {
        self.states
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                SlotState::Mapping(pts) => Some((i, *pts)),
                _ => None,
            })
            .min_by_key(|(_, pts)| *pts)
    }

    pub fn release(&mut self, i: usize) {
        self.states[i] = SlotState::Free;
    }

    pub fn is_idle(&self) -> bool {
        self.states.iter().all(|s| *s == SlotState::Free)
    }
}

pub struct Readback {
    target: RenderTarget,
    buffers: Vec<wgpu::Buffer>,
    mapped: Vec<Arc<AtomicBool>>,
    ring: Ring,
    size: (u32, u32),
}

impl Readback {
    /// A capture texture of `size` in [`CAPTURE_FORMAT`], plus the staging ring.
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let bytes = u64::from(padded_bytes_per_row(size.0)) * u64::from(size.1);
        let buffers = (0..SLOTS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("capture staging"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            target: RenderTarget::new(device, "capture", size.0, size.1, CAPTURE_FORMAT),
            buffers,
            mapped: (0..SLOTS)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            ring: Ring::default(),
            size,
        }
    }

    /// The texture the capture composite draws into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.target.view
    }

    /// Records a copy of the capture texture for frame `pts`. Returns false, recording
    /// nothing, when every staging buffer is still busy.
    pub fn copy(&mut self, encoder: &mut wgpu::CommandEncoder, pts: i64) -> bool {
        let Some(slot) = self.ring.claim(pts) else {
            return false;
        };
        encoder.copy_texture_to_buffer(
            self.target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffers[slot],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row(self.size.0)),
                    rows_per_image: Some(self.size.1),
                },
            },
            self.target.texture.size(),
        );
        true
    }

    /// Starts mapping the buffers copied into since the last call. Call after submitting
    /// the commands that contain the copies.
    pub fn after_submit(&mut self) {
        for slot in self.ring.start_mapping() {
            let mapped = self.mapped[slot].clone();
            mapped.store(false, Ordering::Release);
            self.buffers[slot].map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    mapped.store(true, Ordering::Release);
                }
            });
        }
    }

    /// Whether any frame is still on its way back from the GPU.
    pub fn is_busy(&self) -> bool {
        !self.ring.is_idle()
    }

    /// Returns the frames that have arrived, oldest first. With `wait`, blocks until
    /// everything submitted so far has arrived. `buffer` supplies reusable memory.
    pub fn collect(
        &mut self,
        device: &wgpu::Device,
        wait: bool,
        mut buffer: impl FnMut() -> Vec<u8>,
    ) -> Vec<Frame> {
        let poll = if wait {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        if let Err(err) = device.poll(poll) {
            log::warn!("capture readback poll failed: {err}");
        }
        let mut frames = Vec::new();
        while let Some((slot, pts)) = self.ring.oldest_mapping() {
            if !self.mapped[slot].load(Ordering::Acquire) {
                break;
            }
            let mut rgba = buffer();
            {
                let data = self.buffers[slot]
                    .get_mapped_range(..)
                    .expect("capture buffer is mapped");
                unpad(&data, self.size.0, self.size.1, &mut rgba);
            }
            self.buffers[slot].unmap();
            self.ring.release(slot);
            frames.push(Frame { pts, rgba });
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_used_in_rotation() {
        let mut ring = Ring::default();
        assert_eq!(ring.claim(0), Some(0));
        assert_eq!(ring.claim(1), Some(1));
        assert_eq!(ring.claim(2), Some(2));
        assert_eq!(ring.start_mapping(), [0, 1, 2]);
        ring.release(1);
        assert_eq!(
            ring.claim(3),
            None,
            "slot 0 is next in rotation and still busy"
        );
        ring.release(0);
        assert_eq!(ring.claim(3), Some(0));
        assert_eq!(ring.claim(4), Some(1));
    }

    #[test]
    fn a_full_ring_refuses_new_frames() {
        let mut ring = Ring::default();
        for pts in 0..SLOTS as i64 {
            assert!(ring.claim(pts).is_some());
        }
        assert_eq!(ring.claim(9), None);
        ring.start_mapping();
        let (slot, pts) = ring.oldest_mapping().unwrap();
        assert_eq!((slot, pts), (0, 0));
        ring.release(slot);
        assert_eq!(ring.claim(9), Some(0));
    }

    #[test]
    fn oldest_frame_comes_back_first() {
        let mut ring = Ring::default();
        ring.claim(5);
        ring.claim(6);
        ring.start_mapping();
        ring.release(0);
        ring.claim(7);
        ring.start_mapping();
        assert_eq!(ring.oldest_mapping(), Some((1, 6)));
        assert!(!ring.is_idle());
    }
}
