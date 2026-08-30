//! This crate and `kotor-ncs-isa` must describe the same instruction set.
//!
//! They describe it differently, on purpose. A compiler builds an instruction
//! from two vocabularies — pick an operation, pick an operand typing — so
//! [`bionsscomp::ncs::op`] and [`bionsscomp::ncs::qualifier`] are separate
//! lists, and the engine-structure qualifiers are parametric: structure `n` is
//! `ENGINE + n`. A disassembler goes the other way, matching a byte pair it
//! already has, so `kotor-ncs-isa` enumerates the pairs instead.
//!
//! Neither shape is wrong and neither is derivable from the other without
//! making it worse for its own job, so the two stay separate. What must not
//! happen is the numbers drifting apart. These tests are that guarantee: the
//! shared crate is the authority, and a disagreement fails the build.

use bionsscomp::ncs::{op, qualifier};

/// Every opcode this compiler can emit, with the name it uses for it.
fn compiler_opcodes() -> Vec<(&'static str, u8)> {
    vec![
        ("CPDOWNSP", op::CPDOWNSP),
        ("RSADD", op::RSADD),
        ("CPTOPSP", op::CPTOPSP),
        ("CONST", op::CONST),
        ("ACTION", op::ACTION),
        ("LOGAND", op::LOGAND),
        ("LOGOR", op::LOGOR),
        ("INCOR", op::INCOR),
        ("EXCOR", op::EXCOR),
        ("BOOLAND", op::BOOLAND),
        ("EQUAL", op::EQUAL),
        ("NEQUAL", op::NEQUAL),
        ("GEQ", op::GEQ),
        ("GT", op::GT),
        ("LT", op::LT),
        ("LEQ", op::LEQ),
        ("SHLEFT", op::SHLEFT),
        ("SHRIGHT", op::SHRIGHT),
        ("USHRIGHT", op::USHRIGHT),
        ("ADD", op::ADD),
        ("SUB", op::SUB),
        ("MUL", op::MUL),
        ("DIV", op::DIV),
        ("MOD", op::MOD),
        ("NEG", op::NEG),
        ("COMP", op::COMP),
        ("MOVSP", op::MOVSP),
        ("JMP", op::JMP),
        ("JSR", op::JSR),
        ("JZ", op::JZ),
        ("RETN", op::RETN),
        ("DESTRUCT", op::DESTRUCT),
        ("NOT", op::NOT),
        ("DECISP", op::DECISP),
        ("INCISP", op::INCISP),
        ("JNZ", op::JNZ),
        ("CPDOWNBP", op::CPDOWNBP),
        ("CPTOPBP", op::CPTOPBP),
        ("DECIBP", op::DECIBP),
        ("INCIBP", op::INCIBP),
        ("SAVEBP", op::SAVEBP),
        ("RESTOREBP", op::RESTOREBP),
        ("STORE_STATE", op::STORE_STATE),
    ]
}

/// Every operand typing this compiler names outright.
///
/// The engine-structure qualifiers are left out: they are a base plus an
/// index rather than a fixed list, and [`engine_structure_qualifiers`] covers
/// them.
fn compiler_qualifiers() -> Vec<(&'static str, u8)> {
    vec![
        ("NONE", qualifier::NONE),
        ("STRUCTURAL", qualifier::STRUCTURAL),
        ("INT", qualifier::INT),
        ("FLOAT", qualifier::FLOAT),
        ("STR", qualifier::STR),
        ("OBJECT", qualifier::OBJECT),
        ("ENGINE", qualifier::ENGINE),
        ("INT_INT", qualifier::INT_INT),
        ("FLOAT_FLOAT", qualifier::FLOAT_FLOAT),
        ("OBJECT_OBJECT", qualifier::OBJECT_OBJECT),
        ("STR_STR", qualifier::STR_STR),
        ("AGGREGATE", qualifier::AGGREGATE),
        ("INT_FLOAT", qualifier::INT_FLOAT),
        ("FLOAT_INT", qualifier::FLOAT_INT),
        ("ENGINE_PAIR", qualifier::ENGINE_PAIR),
        ("VECTOR_VECTOR", qualifier::VECTOR_VECTOR),
        ("VECTOR_FLOAT", qualifier::VECTOR_FLOAT),
        ("FLOAT_VECTOR", qualifier::FLOAT_VECTOR),
    ]
}

/// True when `byte` is an engine-structure typing, single or paired.
///
/// The games define a handful of engine structure types; `ENGINE + n` types a
/// single one and `ENGINE_PAIR + n` types two of the same. Four of each are
/// reachable in the shipped instruction set.
fn engine_structure_qualifiers(byte: u8) -> bool {
    const MAX_STRUCTURE: u8 = 3;
    (qualifier::ENGINE..=qualifier::ENGINE + MAX_STRUCTURE).contains(&byte)
        || (qualifier::ENGINE_PAIR..=qualifier::ENGINE_PAIR + MAX_STRUCTURE).contains(&byte)
}

/// `NOP` exists in the bytecode but this compiler never emits one, so it is
/// the one opcode the shared table has that `op` does not name.
const OPCODES_NOT_EMITTED: &[u8] = &[0x2d];

#[test]
fn every_opcode_this_compiler_emits_is_in_the_shared_table() {
    for (name, byte) in compiler_opcodes() {
        let known = kotor_ncs_isa::INSTRUCTIONS
            .iter()
            .any(|ins| ins.opcode == byte);
        assert!(
            known,
            "op::{name} is 0x{byte:02x}, which kotor-ncs-isa does not list as an opcode"
        );
    }
}

#[test]
fn the_shared_table_holds_no_opcode_this_compiler_cannot_name() {
    let named: Vec<u8> = compiler_opcodes().into_iter().map(|(_, b)| b).collect();

    for ins in kotor_ncs_isa::INSTRUCTIONS {
        if named.contains(&ins.opcode) || OPCODES_NOT_EMITTED.contains(&ins.opcode) {
            continue;
        }
        panic!(
            "kotor-ncs-isa lists {} as opcode 0x{:02x}, which `op` does not name. \
             Either add it, or record it in OPCODES_NOT_EMITTED with a reason.",
            ins.mnemonic, ins.opcode
        );
    }
}

#[test]
fn every_qualifier_the_shared_table_uses_is_one_this_compiler_can_name() {
    let named: Vec<u8> = compiler_qualifiers().into_iter().map(|(_, b)| b).collect();

    for ins in kotor_ncs_isa::INSTRUCTIONS {
        let ok = named.contains(&ins.qualifier) || engine_structure_qualifiers(ins.qualifier);
        assert!(
            ok,
            "{} carries qualifier 0x{:02x}, which `qualifier` neither names nor \
             covers as an engine structure typing",
            ins.mnemonic, ins.qualifier
        );
    }
}

#[test]
fn the_parametric_engine_qualifiers_line_up_with_the_enumerated_ones() {
    // RSADDEFF..RSADDTAL and EQUALEFFEFF..EQUALTALTAL are where the shared
    // table spells out what this crate expresses as a base plus an index.
    // If those two ever disagree the compiler would type engine structures
    // one slot off, which nothing else would catch.
    for (mnemonic, expected) in [
        ("RSADDEFF", qualifier::ENGINE),
        ("RSADDEVT", qualifier::ENGINE + 1),
        ("RSADDLOC", qualifier::ENGINE + 2),
        ("RSADDTAL", qualifier::ENGINE + 3),
        ("EQUALEFFEFF", qualifier::ENGINE_PAIR),
        ("EQUALEVTEVT", qualifier::ENGINE_PAIR + 1),
        ("EQUALLOCLOC", qualifier::ENGINE_PAIR + 2),
        ("EQUALTALTAL", qualifier::ENGINE_PAIR + 3),
    ] {
        let ins = kotor_ncs_isa::lookup_mnemonic(mnemonic)
            .unwrap_or_else(|| panic!("kotor-ncs-isa no longer lists {mnemonic}"));
        assert_eq!(
            ins.qualifier, expected,
            "{mnemonic} is qualifier 0x{:02x} in kotor-ncs-isa but 0x{expected:02x} here",
            ins.qualifier
        );
    }
}

#[test]
fn store_state_is_the_same_byte_in_both() {
    // STORE_STATE reuses the engine-structure base as its qualifier, which
    // reads like a copy/paste slip until you check it. Pin it so nobody
    // "fixes" it in one place only.
    let ins = kotor_ncs_isa::lookup_mnemonic("STORE_STATE").expect("STORE_STATE");
    assert_eq!(ins.opcode, op::STORE_STATE);
    assert_eq!(ins.qualifier, qualifier::STORED_STATE);
    assert_eq!(qualifier::STORED_STATE, qualifier::ENGINE);
}
