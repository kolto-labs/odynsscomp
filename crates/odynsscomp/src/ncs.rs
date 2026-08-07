//! The compiled script container: header layout and byte emission.
//!
//! An `.ncs` file is a flat, big-endian byte stream:
//!
//! ```text
//! offset  size  meaning
//! 0       4     "NCS "
//! 4       4     "V1.0"
//! 8       1     0x42, the size marker opcode
//! 9       4     total file size in bytes, big-endian
//! 13      ..    instruction stream
//! ```
//!
//! The header helpers here are real. Instruction encoding is left to whoever
//! lands [`crate::codegen`]; this module only owns the container and the byte
//! writer it hands out.

use crate::error::{CompileError, Diagnostic, Span, Stage};

/// The four bytes that open every compiled script.
pub const MAGIC: [u8; 4] = *b"NCS ";

/// The only script format version either game ships.
pub const VERSION: [u8; 4] = *b"V1.0";

/// The opcode that introduces the total file size.
pub const OP_SIZE: u8 = 0x42;

/// Bytes taken by the magic, version, size marker, and size field together.
pub const HEADER_LEN: usize = 13;

/// The instruction set, as opcode bytes.
///
/// An instruction is one opcode byte, one qualifier byte, then operands. The
/// qualifier says what the operand types are, so `+` is `14 20` over two ints
/// and `14 23` over two strings. [`qualifier`] holds those.
///
/// The names are the ones the reference compiler's enum uses. Two of them
/// mislead: `BOOLAND` is bitwise `&`, and `LOGAND` is `&&`.
pub mod op {
    /// Copy from the top of the stack down to an address. `01 01`.
    pub const CPDOWNSP: u8 = 0x01;
    /// Reserve a zeroed slot for a declaration. `02 <type>`.
    pub const RSADD: u8 = 0x02;
    /// Copy from an address to the top of the stack. `03 01`.
    pub const CPTOPSP: u8 = 0x03;
    /// Push a literal. `04 <type>`.
    pub const CONST: u8 = 0x04;
    /// Call an engine routine. `05 00 <u16 id> <u8 argc>`.
    pub const ACTION: u8 = 0x05;
    /// `&&`, the join once both operands are on the stack.
    pub const LOGAND: u8 = 0x06;
    /// `||`, the join once both operands are on the stack.
    pub const LOGOR: u8 = 0x07;
    /// `|`, bitwise or.
    pub const INCOR: u8 = 0x08;
    /// `^`, bitwise exclusive or.
    pub const EXCOR: u8 = 0x09;
    /// `&`, bitwise and. Named after the wrong operator in the original.
    pub const BOOLAND: u8 = 0x0a;
    /// `==`.
    pub const EQUAL: u8 = 0x0b;
    /// `!=`.
    pub const NEQUAL: u8 = 0x0c;
    /// `>=`.
    pub const GEQ: u8 = 0x0d;
    /// `>`.
    pub const GT: u8 = 0x0e;
    /// `<`.
    pub const LT: u8 = 0x0f;
    /// `<=`.
    pub const LEQ: u8 = 0x10;
    /// `<<`.
    pub const SHLEFT: u8 = 0x11;
    /// `>>`.
    pub const SHRIGHT: u8 = 0x12;
    /// `>>>`, unsigned right shift.
    pub const USHRIGHT: u8 = 0x13;
    /// `+`, including string concatenation.
    pub const ADD: u8 = 0x14;
    /// `-`.
    pub const SUB: u8 = 0x15;
    /// `*`.
    pub const MUL: u8 = 0x16;
    /// `/`.
    pub const DIV: u8 = 0x17;
    /// `%`.
    pub const MOD: u8 = 0x18;
    /// Unary `-`.
    pub const NEG: u8 = 0x19;
    /// Unary `~`.
    pub const COMP: u8 = 0x1a;
    /// Move the stack pointer. `1B 00 <i32 delta>`, always negative.
    pub const MOVSP: u8 = 0x1b;
    /// Unconditional jump. `1D 00 <i32 relative>`.
    pub const JMP: u8 = 0x1d;
    /// Call a script function. `1E 00 <i32 relative>`.
    pub const JSR: u8 = 0x1e;
    /// Pop, and jump if it was zero. `1F 00 <i32 relative>`.
    pub const JZ: u8 = 0x1f;
    /// Return from a routine. `20 00`.
    pub const RETN: u8 = 0x20;
    /// Keep one slice of the top of the stack and drop the rest. `21 01`.
    pub const DESTRUCT: u8 = 0x21;
    /// Unary `!`.
    pub const NOT: u8 = 0x22;
    /// Decrement an `int` local in place. `23 03 <i32 address>`.
    pub const DECISP: u8 = 0x23;
    /// Increment an `int` local in place. `24 03 <i32 address>`.
    pub const INCISP: u8 = 0x24;
    /// Pop, and jump if it was not zero. `25 00 <i32 relative>`.
    pub const JNZ: u8 = 0x25;
    /// Copy from the top of the stack down to a global. `26 01`.
    pub const CPDOWNBP: u8 = 0x26;
    /// Copy a global to the top of the stack. `27 01`.
    pub const CPTOPBP: u8 = 0x27;
    /// Decrement an `int` global in place. `28 03 <i32 address>`.
    pub const DECIBP: u8 = 0x28;
    /// Increment an `int` global in place. `29 03 <i32 address>`.
    pub const INCIBP: u8 = 0x29;
    /// Point the base pointer at the globals. `2A 00`.
    pub const SAVEBP: u8 = 0x2a;
    /// Put the base pointer back. `2B 00`.
    pub const RESTOREBP: u8 = 0x2b;
    /// Snapshot the stack for a delayed call. `2C 10 <u32 globals> <u32 frame>`.
    pub const STORE_STATE: u8 = 0x2c;
}

/// Operand type qualifiers: the second byte of an instruction.
pub mod qualifier {
    /// No operands, or operands whose types cannot vary.
    pub const NONE: u8 = 0x00;
    /// A copy or a destructure, whose operands are sizes rather than types.
    pub const STRUCTURAL: u8 = 0x01;
    /// `int`.
    pub const INT: u8 = 0x03;
    /// `float`.
    pub const FLOAT: u8 = 0x04;
    /// `string`.
    pub const STR: u8 = 0x05;
    /// `object`.
    pub const OBJECT: u8 = 0x06;
    /// The first engine structure type. Structure `n` is `ENGINE + n`.
    pub const ENGINE: u8 = 0x10;
    /// `int` on both sides.
    pub const INT_INT: u8 = 0x20;
    /// `float` on both sides.
    pub const FLOAT_FLOAT: u8 = 0x21;
    /// `object` on both sides.
    pub const OBJECT_OBJECT: u8 = 0x22;
    /// `string` on both sides.
    pub const STR_STR: u8 = 0x23;
    /// Two aggregates of one size, which follows as a `u16` byte count.
    pub const AGGREGATE: u8 = 0x24;
    /// `int` then `float`.
    pub const INT_FLOAT: u8 = 0x25;
    /// `float` then `int`.
    pub const FLOAT_INT: u8 = 0x26;
    /// Engine structure `n` on both sides is `ENGINE_PAIR + n`.
    pub const ENGINE_PAIR: u8 = 0x30;
    /// `vector` on both sides.
    pub const VECTOR_VECTOR: u8 = 0x3a;
    /// `vector` then `float`.
    pub const VECTOR_FLOAT: u8 = 0x3b;
    /// `float` then `vector`.
    pub const FLOAT_VECTOR: u8 = 0x3c;
    /// The offset a `STORE_STATE` snapshot resumes at.
    pub const STORED_STATE: u8 = 0x10;
}

/// A growable big-endian byte sink for the instruction stream.
///
/// Every multi-byte value in a compiled script is big-endian, so this is the
/// only place that byte order needs thinking about.
#[derive(Debug, Clone, Default)]
pub struct NcsWriter {
    bytes: Vec<u8>,
}

impl NcsWriter {
    /// Builds an empty writer.
    pub fn new() -> Self {
        NcsWriter::default()
    }

    /// Returns how many bytes have been written.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns true when nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Writes one byte.
    pub fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Writes a big-endian `u16`.
    pub fn write_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a big-endian `i16`.
    ///
    /// `DESTRUCT`'s middle operand is the only signed 16-bit field in the
    /// format.
    pub fn write_i16(&mut self, value: i16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a big-endian `u32`.
    pub fn write_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a big-endian `i32`.
    pub fn write_i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a big-endian IEEE 754 `f32`.
    pub fn write_f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes raw bytes.
    pub fn write_bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    /// Overwrites four bytes at `offset` with a big-endian `u32`.
    ///
    /// Jump targets are patched this way once their destination is known.
    /// Returns an error diagnostic if the range falls outside what was written.
    pub fn patch_u32(&mut self, offset: usize, value: u32) -> Result<(), Diagnostic> {
        let end = offset.saturating_add(4);
        if end > self.bytes.len() {
            return Err(Diagnostic::error(
                Stage::Ncs,
                Span::default(),
                format!("jump patch at {offset} runs past the end of the code stream"),
            ));
        }
        self.bytes[offset..end].copy_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Returns the instruction bytes without the header.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the instruction bytes without the header.
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }

    /// Prepends the header and returns a complete `.ncs` file.
    ///
    /// The size field counts the header, so a script with no instructions is
    /// [`HEADER_LEN`] bytes long.
    pub fn finish(self) -> Vec<u8> {
        write_with_header(&self.bytes)
    }
}

/// Wraps an instruction stream in a compiled script header.
pub fn write_with_header(code: &[u8]) -> Vec<u8> {
    let total = (HEADER_LEN + code.len()) as u32;
    let mut out = Vec::with_capacity(HEADER_LEN + code.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION);
    out.push(OP_SIZE);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(code);
    out
}

/// What the header of a compiled script says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NcsHeader {
    /// The size the file claims to be, in bytes.
    pub declared_size: u32,
}

impl NcsHeader {
    /// Reads and checks the header of `bytes`.
    ///
    /// Rejects a wrong magic, a wrong version, a missing size marker, a short
    /// file, or a size field that disagrees with the real length.
    pub fn parse(bytes: &[u8]) -> Result<NcsHeader, CompileError> {
        fn bad(message: impl Into<String>) -> CompileError {
            CompileError::from_diagnostic(Diagnostic::error(Stage::Ncs, Span::default(), message))
        }

        if bytes.len() < HEADER_LEN {
            return Err(bad(format!(
                "compiled script is {} bytes, shorter than the {HEADER_LEN} byte header",
                bytes.len()
            )));
        }
        if bytes[0..4] != MAGIC {
            return Err(bad("compiled script does not start with \"NCS \""));
        }
        if bytes[4..8] != VERSION {
            return Err(bad("compiled script is not version V1.0"));
        }
        if bytes[8] != OP_SIZE {
            return Err(bad(format!(
                "expected the size opcode {OP_SIZE:#04x}, found {:#04x}",
                bytes[8]
            )));
        }

        let declared_size = u32::from_be_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]);
        if declared_size as usize != bytes.len() {
            return Err(bad(format!(
                "compiled script claims {declared_size} bytes but is {}",
                bytes.len()
            )));
        }

        Ok(NcsHeader { declared_size })
    }

    /// Returns the instruction stream that follows a valid header.
    pub fn code(bytes: &[u8]) -> Result<&[u8], CompileError> {
        NcsHeader::parse(bytes)?;
        Ok(&bytes[HEADER_LEN..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_script_is_just_a_header() {
        let bytes = NcsWriter::new().finish();
        assert_eq!(bytes.len(), HEADER_LEN);
        assert_eq!(&bytes[0..8], b"NCS V1.0");
        assert_eq!(bytes[8], OP_SIZE);
        let header = NcsHeader::parse(&bytes).expect("header parses");
        assert_eq!(header.declared_size as usize, HEADER_LEN);
    }

    #[test]
    fn header_round_trips_through_a_writer() {
        let mut writer = NcsWriter::new();
        writer.write_u8(0x04);
        writer.write_i32(-1);
        writer.write_f32(1.5);
        let expected_code = writer.as_slice().to_vec();

        let bytes = writer.finish();
        assert_eq!(
            NcsHeader::code(&bytes).expect("code follows"),
            &expected_code[..]
        );
    }

    #[test]
    fn multi_byte_values_are_big_endian() {
        let mut writer = NcsWriter::new();
        writer.write_u32(0x0102_0304);
        assert_eq!(writer.as_slice(), &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn patching_rewrites_a_jump_target() {
        let mut writer = NcsWriter::new();
        writer.write_u8(0x1e);
        writer.write_u32(0);
        writer
            .patch_u32(1, 0xdead_beef)
            .expect("offset is in range");
        assert_eq!(writer.as_slice(), &[0x1e, 0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn patching_past_the_end_is_rejected() {
        let mut writer = NcsWriter::new();
        writer.write_u8(0x1e);
        assert!(writer.patch_u32(4, 0).is_err());
    }

    #[test]
    fn a_lying_size_field_is_rejected() {
        let mut bytes = NcsWriter::new().finish();
        bytes[12] = 0xff;
        assert!(NcsHeader::parse(&bytes).is_err());
    }

    #[test]
    fn a_wrong_magic_is_rejected() {
        let mut bytes = NcsWriter::new().finish();
        bytes[0] = b'X';
        assert!(NcsHeader::parse(&bytes).is_err());
    }
}
