//! End to end checks against the committed fixtures.
//!
//! These run everywhere, with no game files. They pin the shapes that byte
//! equality with the original compiler turns on, and the contract that the
//! library never writes to disk.

use std::path::PathBuf;

use odynsscomp::ncs::{NcsHeader, NcsWriter, HEADER_LEN};
use odynsscomp::{compile, compile_file, CompileOptions, Game, NwScript};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn the_fixtures_are_where_the_tests_expect_them() {
    assert!(fixture("hello.nss").is_file());
    assert!(fixture("nwscript_stub.nss").is_file());
}

#[test]
fn a_fixture_compiles_to_a_well_formed_compiled_script() {
    let nwscript = NwScript::empty(Game::K1);
    let options = CompileOptions::new(&nwscript);

    let bytes = compile_file(fixture("hello.nss"), &options).expect("the fixture compiles");
    let header = NcsHeader::parse(&bytes).expect("the header is well formed");
    assert_eq!(header.declared_size as usize, bytes.len());
    assert!(bytes.len() > HEADER_LEN);
}

#[test]
fn compiling_never_writes_next_to_the_source() {
    let nwscript = NwScript::empty(Game::K2);
    let options = CompileOptions::new(&nwscript);
    let _ = compile_file(fixture("hello.nss"), &options);

    assert!(
        !fixture("hello.ncs").exists(),
        "the library must not write output; that is the caller's job"
    );
}

#[test]
fn a_routine_table_numbers_its_prototypes_by_position() {
    // The stub file is not game data. It has the shape, not the contents.
    let table = NwScript::load(Game::K1, fixture("nwscript_stub.nss")).expect("the table loads");
    assert_eq!(table.routine("PrintString").expect("id 0").id, 0);
    assert_eq!(table.routine("Random").expect("id 1").id, 1);
    assert!(table.constant("TRUE").is_some());
}

#[test]
fn calling_a_routine_emits_an_action_with_the_id_the_table_gave_it() {
    let table = NwScript::load(Game::K1, fixture("nwscript_stub.nss")).expect("the table loads");
    let options = CompileOptions::new(&table);
    let bytes = compile("void main() { PrintString(\"hi\"); }", &options).expect("it compiles");

    // ACTION, no qualifier, routine 0, one argument.
    assert!(
        bytes
            .windows(5)
            .any(|w| w == [0x05, 0x00, 0x00, 0x00, 0x01]),
        "expected ACTION routine=0 argc=1 in {bytes:02x?}"
    );
}

#[test]
fn an_omitted_argument_is_filled_in_from_the_declaration() {
    let table = NwScript::load(Game::K1, fixture("nwscript_stub.nss")).expect("the table loads");
    let options = CompileOptions::new(&table);
    let bytes = compile("void main() { Random(4); }", &options).expect("it compiles");

    // Pushed right to left, so the default nSeed of 0 goes first.
    let pushes = [
        0x04, 0x03, 0x00, 0x00, 0x00, 0x00, // CONSTI 0
        0x04, 0x03, 0x00, 0x00, 0x00, 0x04, // CONSTI 4
        0x05, 0x00, 0x00, 0x01, 0x02, // ACTION routine=1 argc=2
    ];
    assert!(
        bytes.windows(pushes.len()).any(|w| w == pushes),
        "expected the default argument at the call site in {bytes:02x?}"
    );
}

/// A string literal reaches `CONSTS` as the source file's own bytes.
///
/// KOTOR source is Windows-1252, and `CONSTS` counts bytes, so `"café"` is
/// four bytes ending in `0xe9`. Decoding the literal to text on the way through
/// would make it five, or replace the accented byte outright, and every script
/// with one would be byte-wrong.
#[test]
fn a_non_ascii_string_literal_keeps_its_source_bytes() {
    let nwscript = NwScript::empty(Game::K1);
    let options = CompileOptions::new(&nwscript);

    let source =
        odynsscomp::preprocessor::decode_source(b"void main() { string s = \"caf\xe9\"; }");
    let bytes = compile(&source, &options).expect("it compiles");

    let expected = [0x04, 0x05, 0x00, 0x04, b'c', b'a', b'f', 0xe9];
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "expected CONSTS of length 4 ending in 0xe9, got {bytes:02x?}"
    );
}

#[test]
fn a_missing_routine_table_reports_the_path_it_tried() {
    let error = NwScript::load(Game::K2, fixture("no_such_file.nss")).expect_err("it is missing");
    assert!(!error.is_not_implemented());
    assert!(error.to_string().contains("no_such_file.nss"));
}

#[test]
fn compiling_from_memory_needs_no_files_at_all() {
    let nwscript = NwScript::empty(Game::K1);
    let options = CompileOptions::new(&nwscript).with_source_name("k_inline.nss");
    let bytes = compile("void main() { }", &options).expect("it compiles");
    assert_eq!(bytes.len(), HEADER_LEN + 10);
}

/// `object a, b;` declares two names in the scope it was written in.
///
/// One declaration statement per declarator is right, but they cannot be
/// wrapped in a block on the way: a block ends by popping its locals, so the
/// second name would go out of scope on the next line and every use of it
/// would be an error. This is a whole third of the K1 corpus.
#[test]
fn a_declarator_list_leaves_every_name_in_the_surrounding_scope() {
    let nwscript = NwScript::empty(Game::K1);
    let options = CompileOptions::new(&nwscript);
    let bytes = compile("void main() { object oOne, oTwo; oTwo = oOne; }", &options)
        .expect("both names are in scope");

    // Two reservations, then a read and a write. No MOVSP in between.
    let expected = [
        0x02, 0x06, // RSADDO
        0x02, 0x06, // RSADDO
        0x03, 0x01, 0xff, 0xff, 0xff, 0xf8, 0x00, 0x04, // CPTOPSP -8, 4
        0x01, 0x01, 0xff, 0xff, 0xff, 0xf8, 0x00, 0x04, // CPDOWNSP -8, 4
    ];
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "expected two reservations and a copy, got {bytes:02x?}"
    );
}

#[test]
fn an_empty_compiled_script_round_trips_through_the_container() {
    let bytes = NcsWriter::new().finish();
    assert_eq!(bytes.len(), HEADER_LEN);
    assert!(NcsHeader::parse(&bytes).is_ok());
    assert!(NcsHeader::code(&bytes)
        .expect("code follows the header")
        .is_empty());
}
