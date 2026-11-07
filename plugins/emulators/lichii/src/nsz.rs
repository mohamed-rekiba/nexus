//! Solid NSZ files made back into NSPs, for Lichii plays only NSZs compressed in blocks.
//!
//! An NSZ is an NSP (a `PFS0` container) whose NCAs are compressed into NCZs. An NCZ holds the
//! NCA's first 0x4000 bytes as they are (its encrypted header), then `NCZSECTN` and a table of
//! the NCA's sections (offset, size, crypto type, AES key and counter), then the NCA's body,
//! decrypted, as zstd. A solid NCZ has one zstd stream for the whole body; a block one
//! (`NCZBLOCK`) compresses it in blocks, which Lichii reads as it plays.
//!
//! Making the NCA back: copy the header, decompress the body section by section, and encrypt
//! each AES-CTR section again (crypto type 3 or 4) with its key; the counter is the section's,
//! with its last 8 bytes the NCA offset / 16. This is what Lichii's `SolidNczFile` and the
//! `nsz` tool do, so the NCA is the one the NSZ was made from. No console key is needed: the
//! keys are in the NCZ.

use std::io::Read;

use aes::Aes128;
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use emulator_kit::fail::{Context, Error, Result};
use emulator_kit::read_at::ReadAt;

const PFS0: &[u8; 4] = b"PFS0";
/// An NCA's header, which an NCZ keeps as it is.
const NCA_HEADER: u64 = 0x4000;
const SECTIONS_MAGIC: &[u8; 8] = b"NCZSECTN";
const BLOCK_MAGIC: &[u8; 8] = b"NCZBLOCK";
/// The bytes of one section entry.
const SECTION: u64 = SECTION_BYTES as u64;
const SECTION_BYTES: usize = 0x40;
/// The most section entries an NCZ may have (an update's patch data has thousands).
const MAX_SECTIONS: u64 = 0x10000;
/// The most files a PFS0 may have.
const MAX_FILES: u32 = 0x1000;
/// How much is decompressed at a time.
const CHUNK: usize = 1 << 20;
/// zstd's largest window, which `nsz` may use.
const WINDOW_LOG_MAX: u32 = 31;

/// One section of an NCA, as an NCZ lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Section {
    offset: u64,
    size: u64,
    crypto: u64,
    key: [u8; 16],
    counter: [u8; 16],
}

impl Section {
    /// AES-CTR, or BKTR, which is AES-CTR too.
    fn ctr(&self) -> bool {
        self.crypto == 3 || self.crypto == 4
    }
}

/// A solid NCZ: its sections, where its zstd stream starts, and the NCA's size.
#[derive(Clone, Debug)]
struct Solid {
    sections: Vec<Section>,
    payload: u64,
    nca_size: u64,
}

/// A file of the NSP to write: where it comes from in the NSZ, and how.
#[derive(Clone, Debug)]
struct File {
    name: String,
    /// Where its bytes start in the NSZ.
    offset: u64,
    /// Its bytes in the NSZ.
    stored: u64,
    /// `Some` for a solid NCZ, which becomes an NCA.
    solid: Option<Solid>,
}

impl File {
    fn size(&self) -> u64 {
        self.solid.as_ref().map_or(self.stored, |s| s.nca_size)
    }
}

/// What an NSZ becomes: its files, some of them NCAs from solid NCZs.
#[derive(Clone, Debug)]
pub struct Plan {
    files: Vec<File>,
}

impl Plan {
    /// The NSZ in `source`, when it has a solid NCZ to make back. `None` for one Lichii plays
    /// as it is (every NCZ in blocks), and for a file without a PFS0 header, which is copied as
    /// it is for Lichii to judge.
    pub fn of(source: &impl ReadAt) -> Result<Option<Self>> {
        let mut head = [0u8; 16];
        if source.read_exact_at(&mut head, 0).is_err() || &head[..4] != PFS0 {
            return Ok(None);
        }
        let count = u32::from_le_bytes(head[4..8].try_into().expect("4 bytes"));
        let strings = u32::from_le_bytes(head[8..12].try_into().expect("4 bytes"));
        emulator_kit::ensure!(count <= MAX_FILES, "This NSZ lists {count} files");
        let table = 16 + u64::from(count) * 0x18;
        let mut entries = vec![0u8; usize::try_from(table - 16).map_err(Error::msg)?];
        source.read_exact_at(&mut entries, 16).context("Read the NSZ's file table")?;
        let mut names = vec![0u8; strings as usize];
        source.read_exact_at(&mut names, table).context("Read the NSZ's names")?;
        let data = table + u64::from(strings);
        let mut files = Vec::new();
        for entry in entries.as_chunks::<0x18>().0 {
            let le = |at: usize| u64::from_le_bytes(entry[at..at + 8].try_into().expect("8 bytes"));
            let name_at = u32::from_le_bytes(entry[16..20].try_into().expect("4 bytes")) as usize;
            let name = names.get(name_at..).context("A name past the NSZ's names")?;
            let name = String::from_utf8_lossy(&name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())]);
            let (offset, stored) = (data + le(0), le(8));
            let solid = match name.to_ascii_lowercase().ends_with(".ncz") {
                true => solid(source, offset, stored).with_context(|| format!("Read {name}"))?,
                false => None,
            };
            files.push(File { name: name.into_owned(), offset, stored, solid });
        }
        Ok(files.iter().any(|f| f.solid.is_some()).then_some(Self { files }))
    }

    /// The NSP's bytes.
    pub fn size(&self) -> u64 {
        let header = self.header().len() as u64;
        header + self.files.iter().map(File::size).sum::<u64>()
    }

    /// Whether every NCZ became an NCA, so the file is an NSP; else it stays an NSZ, with its
    /// NCZs in blocks.
    pub fn all_nca(&self) -> bool {
        self.files.iter().all(|f| f.solid.is_some() || !f.name.to_ascii_lowercase().ends_with(".ncz"))
    }

    /// A file's name in the NSP: a solid NCZ's is its NCA's.
    fn name(file: &File) -> String {
        match (&file.solid, file.name.strip_suffix(".ncz")) {
            (Some(_), Some(stem)) => format!("{stem}.nca"),
            _ => file.name.clone(),
        }
    }

    /// The NSP's PFS0 header: its file table and names, padded to 0x20 bytes.
    fn header(&self) -> Vec<u8> {
        let mut names = Vec::new();
        let mut entries = Vec::new();
        let mut offset = 0u64;
        for file in &self.files {
            entries.extend_from_slice(&offset.to_le_bytes());
            entries.extend_from_slice(&file.size().to_le_bytes());
            entries.extend_from_slice(&(names.len() as u32).to_le_bytes());
            entries.extend_from_slice(&0u32.to_le_bytes());
            names.extend_from_slice(Self::name(file).as_bytes());
            names.push(0);
            offset += file.size();
        }
        let unpadded = 16 + entries.len() + names.len();
        names.resize(names.len() + (0x20 - unpadded % 0x20) % 0x20, 0);
        let mut out = Vec::with_capacity(16 + entries.len() + names.len());
        out.extend_from_slice(PFS0);
        out.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&entries);
        out.extend_from_slice(&names);
        out
    }

    /// Writes the NSP to `sink`, reading the NSZ from `source`. `cancel` stops it between
    /// chunks.
    pub fn write(
        &self,
        source: &impl ReadAt,
        sink: &mut dyn FnMut(&[u8]) -> Result<()>,
        cancel: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        sink(&self.header())?;
        for file in &self.files {
            match &file.solid {
                Some(solid) => write_nca(source, file, solid, sink, cancel)
                    .with_context(|| format!("Make {}", Self::name(file)))?,
                None => copy(source, file.offset, file.stored, sink, cancel)?,
            }
        }
        Ok(())
    }
}

/// The NCZ at `offset` when it is solid; `None` when it is in blocks.
fn solid(source: &impl ReadAt, offset: u64, stored: u64) -> Result<Option<Solid>> {
    emulator_kit::ensure!(stored > NCA_HEADER + 16, "It is too small for an NCZ");
    let mut head = [0u8; 16];
    source.read_exact_at(&mut head, offset + NCA_HEADER)?;
    emulator_kit::ensure!(&head[..8] == SECTIONS_MAGIC, "It has no NCZSECTN");
    let count = u64::from_le_bytes(head[8..16].try_into().expect("8 bytes"));
    emulator_kit::ensure!(count > 0 && count <= MAX_SECTIONS, "It lists {count} sections");
    let mut table = vec![0u8; usize::try_from(count * SECTION).map_err(Error::msg)?];
    source.read_exact_at(&mut table, offset + NCA_HEADER + 16)?;
    let payload = offset + NCA_HEADER + 16 + count * SECTION;
    let mut magic = [0u8; 8];
    if source.read_exact_at(&mut magic, payload).is_ok() && &magic == BLOCK_MAGIC {
        return Ok(None);
    }
    let mut sections: Vec<Section> = table
        .as_chunks::<SECTION_BYTES>()
        .0
        .iter()
        .map(|e| {
            let le = |at: usize| u64::from_le_bytes(e[at..at + 8].try_into().expect("8 bytes"));
            Section {
                offset: le(0),
                size: le(8),
                crypto: le(16),
                key: e[32..48].try_into().expect("16 bytes"),
                counter: e[48..64].try_into().expect("16 bytes"),
            }
        })
        .collect();
    // The bytes between the header and the first section come in the stream too, as they are.
    if let Some(first) = sections.first()
        && first.offset > NCA_HEADER
    {
        let gap =
            Section { offset: NCA_HEADER, size: first.offset - NCA_HEADER, crypto: 0, key: [0; 16], counter: [0; 16] };
        sections.insert(0, gap);
    }
    let nca_size = sections.iter().map(|s| s.offset + s.size).fold(NCA_HEADER, u64::max);
    Ok(Some(Solid { sections, payload, nca_size }))
}

/// The AES-CTR counter for NCA offset `offset` of a section: its counter, with its last 8 bytes
/// the offset / 16, big-endian (`SetIv` in Lichii, `AESCTR.seek` in nsz).
fn counter_at(counter: &[u8; 16], offset: u64) -> [u8; 16] {
    let mut iv = *counter;
    iv[8..].copy_from_slice(&(offset >> 4).to_be_bytes());
    iv
}

/// AES-128-CTR over `data` from counter `iv`, in place: each 16 bytes take the next counter.
fn ctr(cipher: &Aes128, iv: [u8; 16], data: &mut [u8]) {
    let mut counter = u128::from_be_bytes(iv);
    for block in data.chunks_mut(16) {
        let mut stream = counter.to_be_bytes().into();
        cipher.encrypt_block(&mut stream);
        for (byte, key) in block.iter_mut().zip(stream.iter()) {
            *byte ^= key;
        }
        counter = counter.wrapping_add(1);
    }
}

/// Writes the NCA of a solid NCZ.
fn write_nca(
    source: &impl ReadAt,
    file: &File,
    solid: &Solid,
    sink: &mut dyn FnMut(&[u8]) -> Result<()>,
    cancel: &dyn Fn() -> Result<()>,
) -> Result<()> {
    copy(source, file.offset, NCA_HEADER, sink, cancel)?;
    let stream = At { source, at: solid.payload, end: file.offset + file.stored };
    let mut zstd = zstd::stream::read::Decoder::new(stream).context("Start zstd")?;
    zstd.window_log_max(WINDOW_LOG_MAX).context("Set zstd's window")?;
    let mut buffer = vec![0u8; CHUNK];
    let mut written = NCA_HEADER;
    for section in &solid.sections {
        let begin = section.offset.max(NCA_HEADER);
        let end = (section.offset + section.size).max(begin);
        if end <= written {
            continue;
        }
        emulator_kit::ensure!(begin <= written, "Its sections leave a gap at {written:#x}");
        let cipher = section.ctr().then(|| Aes128::new(&section.key.into()));
        while written < end {
            cancel()?;
            let want = usize::try_from((end - written).min(CHUNK as u64)).map_err(Error::msg)?;
            zstd.read_exact(&mut buffer[..want]).context("Decompress its body")?;
            if let Some(cipher) = &cipher {
                ctr(cipher, counter_at(&section.counter, written), &mut buffer[..want]);
            }
            sink(&buffer[..want])?;
            written += want as u64;
        }
    }
    emulator_kit::ensure!(written == solid.nca_size, "Its body ends at {written:#x}, not {:#x}", solid.nca_size);
    Ok(())
}

/// Copies `len` bytes at `offset` as they are.
fn copy(
    source: &impl ReadAt,
    offset: u64,
    len: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<()>,
    cancel: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0;
    while done < len {
        cancel()?;
        let want = usize::try_from((len - done).min(CHUNK as u64)).map_err(Error::msg)?;
        source.read_exact_at(&mut buffer[..want], offset + done)?;
        sink(&buffer[..want])?;
        done += want as u64;
    }
    Ok(())
}

/// A [`ReadAt`] read in order, from `at` to `end`.
struct At<'a, R: ReadAt> {
    source: &'a R,
    at: u64,
    end: u64,
}

impl<R: ReadAt> Read for At<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let want = usize::try_from((self.end - self.at).min(buf.len() as u64)).unwrap_or(buf.len());
        if want == 0 {
            return Ok(0);
        }
        self.source.read_exact_at(&mut buf[..want], self.at)?;
        self.at += want as u64;
        Ok(want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn aes_ctr_gives_nist_s_answer() {
        // NIST SP 800-38A, F.5.1 CTR-AES128.Encrypt, blocks 1 and 2.
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff").try_into().unwrap();
        let mut data = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        ctr(&Aes128::new(&key.into()), iv, &mut data);
        assert_eq!(data, hex("874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff"));
    }

    #[test]
    fn a_section_s_counter_ends_in_the_offset_over_16() {
        let counter: [u8; 16] = hex("0102030405060708ffffffffffffffff").try_into().unwrap();
        assert_eq!(counter_at(&counter, 0x4000).to_vec(), hex("01020304050607080000000000000400"));
        assert_eq!(counter_at(&counter, 0x1_2345_6780).to_vec(), hex("01020304050607080000000012345678"));
    }

    /// A PFS0 of `files`, as an NSP or NSZ holds them.
    fn pfs0(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut entries = Vec::new();
        let mut offset = 0u64;
        for (name, data) in files {
            entries.extend_from_slice(&offset.to_le_bytes());
            entries.extend_from_slice(&(data.len() as u64).to_le_bytes());
            entries.extend_from_slice(&(names.len() as u32).to_le_bytes());
            entries.extend_from_slice(&[0; 4]);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
            offset += data.len() as u64;
        }
        let mut out = b"PFS0".to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend(entries);
        out.extend(names);
        for (_, data) in files {
            out.extend_from_slice(data);
        }
        out
    }

    /// A solid NCZ of an NCA with `header` and one section from 0x4000 holding `body`, as nsz
    /// writes it: the section's key and counter, and its body decrypted.
    fn solid_ncz(header: &[u8], body: &[u8], crypto: u64, key: [u8; 16], counter: [u8; 16]) -> Vec<u8> {
        let mut out = header.to_vec();
        out.extend_from_slice(b"NCZSECTN");
        out.extend_from_slice(&1u64.to_le_bytes());
        out.extend_from_slice(&NCA_HEADER.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&crypto.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&key);
        out.extend_from_slice(&counter);
        out.extend(zstd::encode_all(body, 3).unwrap());
        out
    }

    fn convert(nsz: &Vec<u8>) -> Vec<u8> {
        let plan = Plan::of(nsz).unwrap().expect("a solid NCZ");
        let mut out = Vec::new();
        plan.write(
            nsz,
            &mut |b| {
                out.extend_from_slice(b);
                Ok(())
            },
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(out.len() as u64, plan.size());
        out
    }

    #[test]
    fn a_solid_nsz_becomes_the_nsp_it_was_made_from() {
        let header = vec![0xA5u8; NCA_HEADER as usize];
        let body: Vec<u8> = (0..0x3000u32).map(|i| (i * 7 % 251) as u8).collect();
        let key = [0x11; 16];
        let counter: [u8; 16] = hex("00000000000000070000000000000000").try_into().unwrap();
        // The NCA as the console has it: its body encrypted, block by block from offset 0x4000.
        let mut encrypted = body.clone();
        ctr(&Aes128::new(&key.into()), hex("00000000000000070000000000000400").try_into().unwrap(), &mut encrypted);
        let nca = [header.clone(), encrypted].concat();
        let ticket = b"ticket bytes".to_vec();
        let nsz = pfs0(&[("abc.ncz", solid_ncz(&header, &body, 3, key, counter)), ("abc.tik", ticket.clone())]);

        let nsp = convert(&nsz);

        assert_eq!(files(&nsp), [("abc.nca".to_owned(), nca), ("abc.tik".to_owned(), ticket)]);
        let header_end = 16 + 2 * 0x18 + u32::from_le_bytes(nsp[8..12].try_into().unwrap()) as usize;
        assert_eq!(header_end % 0x20, 0, "the header ends on 0x20 bytes");
    }

    /// The files of a PFS0, by name, as a reader of NSPs finds them.
    fn files(pfs0: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u32_at = |at: usize| u32::from_le_bytes(pfs0[at..at + 4].try_into().unwrap()) as usize;
        let u64_at = |at: usize| u64::from_le_bytes(pfs0[at..at + 8].try_into().unwrap()) as usize;
        let (count, strings) = (u32_at(4), u32_at(8));
        let names = 16 + count * 0x18;
        let data = names + strings;
        (0..count)
            .map(|i| {
                let entry = 16 + i * 0x18;
                let name = &pfs0[names + u32_at(entry + 16)..];
                let name = String::from_utf8(name[..name.iter().position(|b| *b == 0).unwrap()].to_vec()).unwrap();
                let start = data + u64_at(entry);
                (name, pfs0[start..start + u64_at(entry + 8)].to_vec())
            })
            .collect()
    }

    #[test]
    fn an_nsz_in_blocks_or_without_ncz_is_left_alone() {
        let header = vec![0u8; NCA_HEADER as usize];
        let mut block = solid_ncz(&header, &[1, 2, 3], 1, [0; 16], [0; 16]);
        let payload = NCA_HEADER as usize + 16 + SECTION as usize;
        block.splice(payload..payload, b"NCZBLOCK".iter().copied());
        assert!(Plan::of(&pfs0(&[("a.ncz", block)])).unwrap().is_none());
        assert!(Plan::of(&pfs0(&[("a.nca", vec![9; 32])])).unwrap().is_none());
        assert!(Plan::of(&b"not a PFS0".to_vec()).unwrap().is_none(), "copied as it is");
    }
}
