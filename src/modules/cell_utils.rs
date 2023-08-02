use std::collections::HashSet;

use anyhow::{Context as _, Result};
use everscale_types::cell::{MAX_BIT_LEN, MAX_REF_COUNT};
use everscale_types::prelude::*;
use num_bigint::{BigInt, Sign};
use num_traits::Zero;
use sha2::Digest;

use crate::core::*;
use crate::util::*;

pub struct CellUtils;

#[fift_module]
impl CellUtils {
    // === Cell builder manipulation ===

    #[cmd(name = "<b", stack)]
    fn interpret_empty(stack: &mut Stack) -> Result<()> {
        stack.push(CellBuilder::new())
    }

    #[cmd(name = "i,", stack, args(signed = true))]
    #[cmd(name = "u,", stack, args(signed = false))]
    fn interpret_store(stack: &mut Stack, signed: bool) -> Result<()> {
        let bits = stack.pop_smallint_range(0, 1023)? as u16;
        let mut int = stack.pop_int()?;
        let mut builder = stack.pop_builder()?;
        store_int_to_builder(&mut builder, &mut int, bits, signed)?;
        stack.push_raw(builder)
    }

    #[cmd(name = "ref,", stack)]
    fn interpret_store_ref(stack: &mut Stack) -> Result<()> {
        let cell = stack.pop_cell()?;
        let mut builder = stack.pop_builder()?;
        builder.store_reference(*cell)?;
        stack.push_raw(builder)
    }

    #[cmd(name = "$,", stack)]
    fn interpret_store_str(stack: &mut Stack) -> Result<()> {
        let string = stack.pop_string()?;
        let mut builder = stack.pop_builder()?;
        builder.store_raw(string.as_bytes(), len_as_bits("string", &*string)?)?;
        stack.push_raw(builder)
    }

    #[cmd(name = "B,", stack)]
    fn interpret_store_bytes(stack: &mut Stack) -> Result<()> {
        let bytes = stack.pop_bytes()?;
        let mut builder = stack.pop_builder()?;
        builder.store_raw(bytes.as_slice(), len_as_bits("byte string", &*bytes)?)?;
        stack.push_raw(builder)
    }

    #[cmd(name = "s,", stack)]
    fn interpret_store_cellslice(stack: &mut Stack) -> Result<()> {
        let slice = stack.pop_slice()?;
        let mut builder = stack.pop_builder()?;
        builder.store_slice(slice.pin())?;
        stack.push_raw(builder)
    }

    #[cmd(name = "sr,", stack)]
    fn interpret_store_cellslice_ref(stack: &mut Stack) -> Result<()> {
        let slice = stack.pop_slice()?;
        let cell = {
            let mut builder = CellBuilder::new();
            builder.store_slice(slice.pin())?;
            builder.build()?
        };
        let mut builder = stack.pop_builder()?;
        builder.store_reference(cell)?;
        stack.push_raw(builder)
    }

    #[cmd(name = "b>", stack, args(is_exotic = false))]
    #[cmd(name = "b>spec", stack, args(is_exotic = true))]
    fn interpret_store_end(stack: &mut Stack, is_exotic: bool) -> Result<()> {
        let mut item = stack.pop_builder()?;
        item.set_exotic(is_exotic);
        let cell = item.build()?;
        stack.push(cell)
    }

    #[cmd(name = "$>s", stack)]
    fn interpret_string_to_cellslice(stack: &mut Stack) -> Result<()> {
        let string = stack.pop_string()?;
        let mut builder = CellBuilder::new();
        builder.store_raw(string.as_bytes(), len_as_bits("slice", &*string)?)?;
        let slice = OwnedCellSlice::new(builder.build()?)?;
        stack.push(slice)
    }

    #[cmd(name = "|+", stack)]
    fn interpret_concat_cellslice(stack: &mut Stack) -> Result<()> {
        let cs2 = stack.pop_slice()?;
        let cs1 = stack.pop_slice()?;
        stack.push({
            let mut builder = CellBuilder::new();
            builder.store_slice(cs1.pin())?;
            builder.store_slice(cs2.pin())?;
            OwnedCellSlice::new(builder.build()?)?
        })
    }

    #[cmd(name = "|_", stack)]
    fn interpret_concat_cellslice_ref(stack: &mut Stack) -> Result<()> {
        let cs2 = stack.pop_slice()?;
        let cs1 = stack.pop_slice()?;

        let cell = {
            let mut builder = CellBuilder::new();
            builder.store_slice(cs2.pin())?;
            builder.build()?
        };

        stack.push({
            let mut builder = CellBuilder::new();
            builder.store_slice(cs1.pin())?;
            builder.store_reference(cell)?;
            OwnedCellSlice::new(builder.build()?)?
        })
    }

    #[cmd(name = "b+", stack)]
    fn interpret_concat_builders(stack: &mut Stack) -> Result<()> {
        let cb2 = stack.pop_builder()?;
        let mut cb1 = stack.pop_builder()?;
        cb1.store_raw(cb2.raw_data(), cb2.bit_len())?;
        for cell in cb2.references() {
            cb1.store_reference(cell.clone())?;
        }
        stack.push_raw(cb1)
    }

    #[cmd(name = "bbits", stack, args(bits = true, refs = false))]
    #[cmd(name = "brefs", stack, args(bits = false, refs = true))]
    #[cmd(name = "bbitrefs", stack, args(bits = true, refs = true))]
    fn interpret_builder_bitrefs(stack: &mut Stack, bits: bool, refs: bool) -> Result<()> {
        let cb = stack.pop_builder()?;
        if bits {
            stack.push_int(cb.bit_len())?;
        }
        if refs {
            stack.push_int(cb.references().len())?;
        }
        Ok(())
    }

    #[cmd(name = "brembits", stack, args(bits = true, refs = false))]
    #[cmd(name = "bremrefs", stack, args(bits = false, refs = true))]
    #[cmd(name = "brembitrefs", stack, args(bits = true, refs = true))]
    fn interpret_builder_rem_bitrefs(stack: &mut Stack, bits: bool, refs: bool) -> Result<()> {
        let cb = stack.pop_builder()?;
        if bits {
            stack.push_int(MAX_BIT_LEN - cb.bit_len())?;
        }
        if refs {
            stack.push_int(MAX_REF_COUNT - cb.references().len())?;
        }
        Ok(())
    }

    #[cmd(name = "hash", stack, args(as_uint = true))]
    #[cmd(name = "hashu", stack, args(as_uint = true))]
    #[cmd(name = "hashB", stack, args(as_uint = false))]
    fn interpret_cell_hash(stack: &mut Stack, as_uint: bool) -> Result<()> {
        let bytes = stack.pop_bytes()?;
        let hash = sha2::Sha256::digest(*bytes);
        if as_uint {
            stack.push(BigInt::from_bytes_be(Sign::Plus, &hash))
        } else {
            stack.push(hash.to_vec())
        }
    }

    // === Cell slice manipulation ===

    #[cmd(name = "<s", stack)]
    fn interpret_from_cell(stack: &mut Stack) -> Result<()> {
        let item = stack.pop_cell()?;
        let slice = OwnedCellSlice::new(*item)?;
        stack.push(slice)
    }

    #[cmd(name = "i@", stack, args(sgn = true, advance = false, quiet = false))]
    #[cmd(name = "u@", stack, args(sgn = false, advance = false, quiet = false))]
    #[cmd(name = "i@+", stack, args(sgn = true, advance = true, quiet = false))]
    #[cmd(name = "u@+", stack, args(sgn = false, advance = true, quiet = false))]
    #[cmd(name = "i@?", stack, args(sgn = true, advance = false, quiet = true))]
    #[cmd(name = "u@?", stack, args(sgn = false, advance = false, quiet = true))]
    #[cmd(name = "i@?+", stack, args(sgn = true, advance = true, quiet = true))]
    #[cmd(name = "u@?+", stack, args(sgn = false, advance = true, quiet = true))]
    fn interpret_load(stack: &mut Stack, sgn: bool, advance: bool, quiet: bool) -> Result<()> {
        let bits = stack.pop_smallint_range(0, 256 + sgn as u32)? as u16;
        let mut raw_cs = stack.pop_slice()?;
        let cs = raw_cs.pin_mut();

        let int = match bits {
            0 => Ok(BigInt::zero()),
            0..=64 if !sgn => cs.get_uint(0, bits).map(BigInt::from),
            0..=64 if sgn => cs.get_uint(0, bits).map(|mut int| {
                if bits < 64 {
                    // Clone sign bit into all high bits
                    int |= ((int >> (bits - 1)) * u64::MAX) << (bits - 1);
                }
                BigInt::from(int as i64)
            }),
            _ => {
                let align = 8 - bits % 8;
                let mut buffer = [0u8; 33];
                cs.get_raw(0, &mut buffer, bits).map(|buffer| {
                    let mut int = if sgn {
                        BigInt::from_signed_bytes_be(buffer)
                    } else {
                        BigInt::from_bytes_be(Sign::Plus, buffer)
                    };
                    int >>= align;
                    int
                })
            }
        };
        let is_ok = int.is_ok();

        match int {
            Ok(int) => {
                stack.push_int(int)?;
                if advance {
                    cs.try_advance(bits, 0);
                    stack.push_raw(raw_cs)?;
                }
            }
            Err(e) if !quiet => return Err(e.into()),
            _ => {}
        }

        if quiet {
            stack.push_bool(is_ok)?;
        }
        Ok(())
    }

    #[cmd(name = "$@", stack, args(s = true, advance = false, quiet = false))]
    #[cmd(name = "B@", stack, args(s = false, advance = false, quiet = false))]
    #[cmd(name = "$@+", stack, args(s = true, advance = true, quiet = false))]
    #[cmd(name = "B@+", stack, args(s = false, advance = true, quiet = false))]
    #[cmd(name = "$@?", stack, args(s = true, advance = false, quiet = true))]
    #[cmd(name = "B@?", stack, args(s = false, advance = false, quiet = true))]
    #[cmd(name = "$@?+", stack, args(s = true, advance = true, quiet = true))]
    #[cmd(name = "B@?+", stack, args(s = false, advance = true, quiet = true))]
    fn interpret_load_bytes(stack: &mut Stack, s: bool, advance: bool, quiet: bool) -> Result<()> {
        let bits = stack.pop_smallint_range(0, 127)? as u16 * 8;
        let mut cs = stack.pop_slice()?;

        let mut buffer = [0; 128];
        let bytes = cs.pin_mut().get_raw(0, &mut buffer, bits);
        let is_ok = bytes.is_ok();

        match bytes {
            Ok(bytes) => {
                let bytes = bytes.to_owned();
                if s {
                    let string = String::from_utf8(bytes)?;
                    stack.push(string)?;
                } else {
                    stack.push(bytes)?;
                }

                if advance {
                    cs.pin_mut().try_advance(bits, 0);
                    stack.push_raw(cs)?;
                }
            }
            Err(e) if !quiet => return Err(e.into()),
            _ => {}
        }

        if quiet {
            stack.push_bool(is_ok)?;
        }
        Ok(())
    }

    #[cmd(name = "ref@", stack, args(advance = false, quiet = false))]
    #[cmd(name = "ref@+", stack, args(advance = true, quiet = false))]
    #[cmd(name = "ref@?", stack, args(advance = false, quiet = true))]
    #[cmd(name = "ref@?+", stack, args(advance = true, quiet = true))]
    fn interpret_load_ref(stack: &mut Stack, advance: bool, quiet: bool) -> Result<()> {
        let mut cs = stack.pop_slice()?;

        let cell = cs.pin_mut().get_reference_cloned(0);
        let is_ok = cell.is_ok();

        match cell {
            Ok(cell) => {
                stack.push(cell)?;
                if advance {
                    cs.pin_mut().try_advance(0, 1);
                    stack.push_raw(cs)?;
                }
            }
            Err(e) if !quiet => return Err(e.into()),
            _ => {}
        }

        if quiet {
            stack.push_bool(is_ok)?;
        }
        Ok(())
    }

    #[cmd(name = "empty?", stack)]
    fn interpret_cell_empty(stack: &mut Stack) -> Result<()> {
        let cs = stack.pop_slice()?;
        stack.push_bool(cs.pin().is_data_empty() && cs.pin().is_refs_empty())
    }

    #[cmd(name = "sbits", stack, args(bits = true, refs = false))]
    #[cmd(name = "srefs", stack, args(bits = false, refs = true))]
    #[cmd(name = "sbitrefs", stack, args(bits = true, refs = true))]
    #[cmd(name = "remaining", stack, args(bits = true, refs = true))]
    fn interpret_slice_bitrefs(stack: &mut Stack, bits: bool, refs: bool) -> Result<()> {
        let cs = stack.pop_slice()?;
        if bits {
            stack.push_int(cs.pin().remaining_bits())?;
        }
        if refs {
            stack.push_int(cs.pin().remaining_refs())?;
        }
        Ok(())
    }

    #[cmd(name = "s>", stack)]
    fn interpret_cell_check_empty(stack: &mut Stack) -> Result<()> {
        let item = stack.pop_slice()?;
        let item = item.pin();
        anyhow::ensure!(
            item.is_data_empty() && item.is_refs_empty(),
            "Expected empty cell slice"
        );
        Ok(())
    }

    #[cmd(name = "totalcsize", stack, args(load_slice = false))]
    #[cmd(name = "totalssize", stack, args(load_slice = true))]
    fn interpret_cell_datasize(stack: &mut Stack, load_slice: bool) -> Result<()> {
        const LIMIT: usize = 1 << 22;
        let (cells, bits, refs) = if load_slice {
            let slice = stack.pop_slice()?;
            StorageStat::compute_for_slice(slice.pin(), LIMIT)
        } else {
            let cell = stack.pop_cell()?;
            StorageStat::compute_for_cell(&**cell, LIMIT)
        }
        .context("Storage compute depth limit reached")?;
        stack.push_int(cells)?;
        stack.push_int(bits)?;
        stack.push_int(refs)
    }

    // === BOC manipulation ===

    #[cmd(name = "B>boc", stack)]
    fn interpret_boc_deserialize(stack: &mut Stack) -> Result<()> {
        let bytes = stack.pop_bytes()?;
        let cell = Boc::decode(*bytes)?;
        stack.push(cell)
    }

    #[cmd(name = "base64>boc", stack)]
    fn interpret_boc_deserialize_base64(stack: &mut Stack) -> Result<()> {
        let bytes = stack.pop_string()?;
        let cell = Boc::decode_base64(*bytes)?;
        stack.push(cell)
    }

    #[cmd(name = "boc>B", stack, args(ext = false, base64 = false))]
    #[cmd(name = "boc>base64", stack, args(ext = false, base64 = true))]
    #[cmd(name = "boc+>B", stack, args(ext = true, base64 = false))]
    #[cmd(name = "boc+>base64", stack, args(ext = true, base64 = true))]
    fn interpret_boc_serialize_ext(stack: &mut Stack, ext: bool, base64: bool) -> Result<()> {
        use everscale_types::boc::ser::BocHeader;

        const MODE_WITH_CRC: u32 = 0b00010;
        const SUPPORTED_MODES: u32 = MODE_WITH_CRC;

        let mode = if ext {
            stack.pop_smallint_range(0, 31)?
        } else {
            0
        };

        anyhow::ensure!(
            mode & !SUPPORTED_MODES == 0,
            "Unsupported BOC serialization mode 0x{mode:x}"
        );

        let cell = stack.pop_cell()?;

        let mut result = Vec::new();
        BocHeader::<ahash::RandomState>::new(&**cell)
            .with_crc(mode & MODE_WITH_CRC != 0)
            .encode(&mut result);

        if base64 {
            stack.push(encode_base64(result))
        } else {
            stack.push(result)
        }
    }

    // === Prefix commands ===

    #[cmd(name = "x{", active, without_space)]
    fn interpret_bitstring_hex_literal(ctx: &mut Context) -> Result<()> {
        let s = ctx.input.scan_until('}')?;
        let builder = decode_hex_bitstring(s.data)?.build()?;
        ctx.stack.push(OwnedCellSlice::new(builder)?)?;
        ctx.stack.push_argcount(1, ctx.dictionary.make_nop())
    }

    #[cmd(name = "b{", active, without_space)]
    fn interpret_bitstring_binary_literal(ctx: &mut Context) -> Result<()> {
        let s = ctx.input.scan_until('}')?;
        let builder = decode_binary_bitstring(s.data)?.build()?;
        ctx.stack.push(OwnedCellSlice::new(builder)?)?;
        ctx.stack.push_argcount(1, ctx.dictionary.make_nop())
    }
}

struct StorageStat<'a> {
    visited: HashSet<&'a HashBytes, ahash::RandomState>,
    cells: u64,
    bits: u64,
    refs: u64,
    limit: usize,
}

impl<'a> StorageStat<'a> {
    fn with_limit(limit: usize) -> Self {
        Self {
            visited: Default::default(),
            cells: 0,
            bits: 0,
            refs: 0,
            limit,
        }
    }

    fn compute_for_slice<'b: 'a>(
        slice: &'a CellSlice<'b>,
        limit: usize,
    ) -> Option<(u64, u64, u64)> {
        let mut this = Self::with_limit(limit);
        if this.add_slice(slice) {
            Some((this.cells, this.bits, this.refs))
        } else {
            None
        }
    }

    fn compute_for_cell(cell: &'a DynCell, limit: usize) -> Option<(u64, u64, u64)> {
        let mut this = Self::with_limit(limit);
        if this.add_cell(cell) {
            Some((this.cells, this.bits, this.refs))
        } else {
            None
        }
    }

    fn add_slice<'b: 'a>(&mut self, slice: &'a CellSlice<'b>) -> bool {
        self.bits = self.bits.saturating_add(slice.remaining_bits() as u64);
        self.refs = self.refs.saturating_add(slice.remaining_refs() as u64);

        for cell in slice.references() {
            if !self.add_cell(cell) {
                return false;
            }
        }

        true
    }

    fn add_cell(&mut self, cell: &'a DynCell) -> bool {
        if !self.visited.insert(cell.repr_hash()) {
            return false;
        }
        if self.cells >= self.limit as u64 {
            return false;
        }

        self.cells += 1;
        self.bits = self.bits.saturating_add(cell.bit_len() as u64);
        self.refs = self.refs.saturating_add(cell.reference_count() as u64);

        for cell in cell.references() {
            if !self.add_cell(cell) {
                return false;
            }
        }

        true
    }
}

fn len_as_bits<T: AsRef<[u8]>>(name: &str, data: T) -> Result<u16> {
    let bits = data.as_ref().len() * 8;
    anyhow::ensure!(
        bits <= everscale_types::cell::MAX_BIT_LEN as usize,
        "{name} does not fit into cell"
    );
    Ok(bits as u16)
}
