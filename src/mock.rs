//! In-memory mock flash for host testing and simulation.

use embedded_storage::nor_flash::{ErrorType, NorFlashError, NorFlashErrorKind};
use embedded_storage_async::nor_flash::{
    NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockFlashError {
    OutOfBounds,
    NotAligned,
}

impl NorFlashError for MockFlashError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
            Self::NotAligned => NorFlashErrorKind::NotAligned,
        }
    }
}

/// Simulated in-memory NOR flash.
pub struct MockFlash<const CAPACITY: usize, const ERASE_SIZE: usize = 4096> {
    data: [u8; CAPACITY],
}

impl<const CAPACITY: usize, const ERASE_SIZE: usize> MockFlash<CAPACITY, ERASE_SIZE> {
    pub const fn new() -> Self {
        Self {
            data: [0xFF; CAPACITY],
        }
    }

    pub fn data(&self) -> &[u8; CAPACITY] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8; CAPACITY] {
        &mut self.data
    }
}

impl<const CAPACITY: usize, const ERASE_SIZE: usize> ErrorType
    for MockFlash<CAPACITY, ERASE_SIZE>
{
    type Error = MockFlashError;
}

impl<const CAPACITY: usize, const ERASE_SIZE: usize> AsyncReadNorFlash
    for MockFlash<CAPACITY, ERASE_SIZE>
{
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        if offset + bytes.len() > CAPACITY {
            return Err(MockFlashError::OutOfBounds);
        }
        bytes.copy_from_slice(&self.data[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        CAPACITY
    }
}

impl<const CAPACITY: usize, const ERASE_SIZE: usize> AsyncNorFlash
    for MockFlash<CAPACITY, ERASE_SIZE>
{
    const WRITE_SIZE: usize = 1;
    const ERASE_SIZE: usize = ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let from = from as usize;
        let to = to as usize;
        if to > CAPACITY || from > to {
            return Err(MockFlashError::OutOfBounds);
        }
        if from % ERASE_SIZE != 0 || to % ERASE_SIZE != 0 {
            return Err(MockFlashError::NotAligned);
        }
        self.data[from..to].fill(0xFF);
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        if offset + bytes.len() > CAPACITY {
            return Err(MockFlashError::OutOfBounds);
        }
        // Emulate NOR flash: bits can only be cleared (ANDed)
        for (i, &b) in bytes.iter().enumerate() {
            self.data[offset + i] &= b;
        }
        Ok(())
    }
}

// Blocking traits
impl<const CAPACITY: usize, const ERASE_SIZE: usize> embedded_storage::nor_flash::ReadNorFlash
    for MockFlash<CAPACITY, ERASE_SIZE>
{
    const READ_SIZE: usize = 1;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        if offset + bytes.len() > CAPACITY {
            return Err(MockFlashError::OutOfBounds);
        }
        bytes.copy_from_slice(&self.data[offset..offset + bytes.len()]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        CAPACITY
    }
}

impl<const CAPACITY: usize, const ERASE_SIZE: usize> embedded_storage::nor_flash::NorFlash
    for MockFlash<CAPACITY, ERASE_SIZE>
{
    const WRITE_SIZE: usize = 1;
    const ERASE_SIZE: usize = ERASE_SIZE;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let from = from as usize;
        let to = to as usize;
        if to > CAPACITY || from > to {
            return Err(MockFlashError::OutOfBounds);
        }
        if from % ERASE_SIZE != 0 || to % ERASE_SIZE != 0 {
            return Err(MockFlashError::NotAligned);
        }
        self.data[from..to].fill(0xFF);
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        let offset = offset as usize;
        if offset + bytes.len() > CAPACITY {
            return Err(MockFlashError::OutOfBounds);
        }
        for (i, &b) in bytes.iter().enumerate() {
            self.data[offset + i] &= b;
        }
        Ok(())
    }
}
