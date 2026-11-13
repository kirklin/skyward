//! Legacy LZ4 has 8 MiB decoded blocks, always compressed even when that
//! makes them larger. FrameDecoder's modern-frame compressed-size bound
//! therefore rejects valid legacy ramdisks containing incompressible data.

use std::io::{self, ErrorKind, Read};

const MAGIC: u32 = 0x184c_2102;
const BLOCK: usize = 8 << 20;
// LZ4_compressBound(BLOCK), not the maximum decoded length.
const COMPRESSED: usize = BLOCK + BLOCK / 255 + 16;

pub struct LegacyDecoder<R> {
    input: R,
    started: bool,
    ended: bool,
    failed: bool,
    encoded: Vec<u8>,
    decoded: Vec<u8>,
    at: usize,
}

impl<R: Read> LegacyDecoder<R> {
    pub fn new(input: R) -> Self {
        Self { input, started: false, ended: false, failed: false, encoded: Vec::new(), decoded: Vec::new(), at: 0 }
    }

    fn next_size(&mut self) -> io::Result<Option<u32>> {
        let mut bytes = [0; 4];
        loop {
            match self.input.read(&mut bytes[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.input.read_exact(&mut bytes[1..])?;
        Ok(Some(u32::from_le_bytes(bytes)))
    }

    fn read_block(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !self.started {
            let mut magic = [0; 4];
            self.input.read_exact(&mut magic)?;
            if u32::from_le_bytes(magic) != MAGIC {
                return Err(io::Error::new(ErrorKind::InvalidData, "invalid legacy LZ4 magic"));
            }
            self.started = true;
        }
        while self.at == self.decoded.len() {
            if self.ended {
                return Ok(0);
            }
            let size = match self.next_size()? {
                None | Some(0) => {
                    self.ended = true;
                    return Ok(0);
                }
                Some(MAGIC) => continue,
                Some(size) => size as usize,
            };
            if size > COMPRESSED {
                return Err(io::Error::new(ErrorKind::InvalidData, "legacy LZ4 compressed block exceeds compressBound(8 MiB)"));
            }
            self.encoded.resize(size, 0);
            self.input.read_exact(&mut self.encoded)?;
            self.decoded.resize(BLOCK, 0);
            let len = lz4_flex::block::decompress_into(&self.encoded, &mut self.decoded)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            self.decoded.truncate(len);
            self.at = 0;
        }
        let len = out.len().min(self.decoded.len() - self.at);
        out[..len].copy_from_slice(&self.decoded[self.at..self.at + len]);
        self.at += len;
        Ok(len)
    }
}

impl<R: Read> Read for LegacyDecoder<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.failed {
            return Err(io::Error::new(ErrorKind::InvalidData, "legacy LZ4 decoder stopped after an error"));
        }
        let result = self.read_block(out);
        self.failed = result.is_err();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::process::{Command, Stdio};

    fn literal(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0xf0];
        let mut rest = data.len() - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
        out.extend_from_slice(data);
        out
    }

    fn stream(data: &[u8]) -> Vec<u8> {
        let block = literal(data);
        let mut out = MAGIC.to_le_bytes().to_vec();
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(&block);
        out
    }

    #[test]
    fn a_valid_legacy_compressed_block_can_exceed_eight_mib() {
        let input = vec![0x5a; BLOCK];
        let packed = stream(&input);
        let size = u32::from_le_bytes(packed[4..8].try_into().unwrap()) as usize;
        assert!(size > BLOCK && size <= COMPRESSED);
        let mut back = Vec::new();
        LegacyDecoder::new(packed.as_slice()).read_to_end(&mut back).unwrap();
        assert_eq!(back, input);

        let mut child = Command::new("lz4").args(["-d", "-c"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(&packed));
        let result = child.wait_with_output().unwrap();
        writer.join().unwrap().unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, input, "the reference decoder accepts the same oversized-on-wire block");
    }

    #[test]
    fn legacy_blocks_frames_and_kernel_zero_terminator_are_supported() {
        let input = vec![7; BLOCK];
        let mut packed = stream(&input);
        packed.extend(stream(&[9; 30]));
        packed.extend_from_slice(&0_u32.to_le_bytes());
        let mut back = Vec::new();
        LegacyDecoder::new(packed.as_slice()).read_to_end(&mut back).unwrap();
        assert_eq!(&back[..BLOCK], input);
        assert_eq!(&back[BLOCK..], &[9; 30]);
    }

    #[test]
    fn corrupt_and_truncated_inputs_fail_without_losing_the_error() {
        let valid = stream(&[5; 30]);
        let mut oversize = MAGIC.to_le_bytes().to_vec();
        oversize.extend_from_slice(&((COMPRESSED + 1) as u32).to_le_bytes());
        let mut overflow = MAGIC.to_le_bytes().to_vec();
        let expanded = vec![0; BLOCK + 1];
        let too_large = literal(&expanded);
        overflow.extend_from_slice(&(too_large.len() as u32).to_le_bytes());
        overflow.extend_from_slice(&too_large);
        for data in [b"xxxx".to_vec(), valid[..3].to_vec(), valid[..6].to_vec(), valid[..valid.len() - 1].to_vec(), oversize, overflow] {
            let mut decoder = LegacyDecoder::new(data.as_slice());
            assert!(decoder.read_to_end(&mut Vec::new()).is_err());
            assert!(decoder.read(&mut [0; 1]).is_err());
            assert_eq!(decoder.read(&mut []).unwrap(), 0);
        }
    }

    #[test]
    fn short_reads_and_empty_output_do_not_consume_or_truncate_blocks() {
        struct OneByte(Cursor<Vec<u8>>);
        impl Read for OneByte {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                let len = out.len().min(1);
                self.0.read(&mut out[..len])
            }
        }
        let mut decoder = LegacyDecoder::new(OneByte(Cursor::new(stream(&[4; 30]))));
        assert_eq!(decoder.read(&mut []).unwrap(), 0);
        let mut back = Vec::new();
        decoder.read_to_end(&mut back).unwrap();
        assert_eq!(back, [4; 30]);
    }

    #[test]
    fn an_error_after_delivered_output_stays_an_error() {
        let mut packed = stream(&[3; 30]);
        packed.extend_from_slice(&[1, 2]);
        let mut decoder = LegacyDecoder::new(packed.as_slice());
        let mut back = Vec::new();
        assert!(decoder.read_to_end(&mut back).is_err());
        assert_eq!(back, [3; 30]);
        assert!(decoder.read(&mut [0; 1]).is_err());
    }
}
