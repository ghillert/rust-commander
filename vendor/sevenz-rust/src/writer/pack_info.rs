use super::*;

#[derive(Debug, Default, Clone)]
pub struct PackInfo {
    pub crcs: Vec<u32>,
    pub sizes: Vec<u64>,
    pub pos: u64,
}

impl PackInfo {
    pub fn write_to<H: Write>(&mut self, header: &mut H) -> std::io::Result<()> {
        header.write_u8(K_PACK_INFO)?;
        write_u64(header, self.pos)?;
        write_u64(header, self.len() as u64)?;
        header.write_u8(K_SIZE)?;
        for size in &self.sizes {
            write_u64(header, *size)?;
        }
        header.write_u8(K_CRC)?;
        // Every added stream has a calculated CRC; zero is a valid value.
        header.write_u8(1)?;
        for crc in &self.crcs {
            header.write_u32::<LittleEndian>(*crc)?;
        }

        header.write_u8(K_END)?;
        Ok(())
    }
}

impl PackInfo {
    #[inline]
    pub fn add_stream(&mut self, size: u64, crc: u32) {
        self.sizes.push(size);
        self.crcs.push(crc);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.sizes.len()
    }
}
