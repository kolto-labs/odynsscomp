//! End to end checks against the committed fixtures.
//!
//! These run everywhere, with no game files. They cover the parts of the crate
//! that are finished and pin the contract the unfinished parts must honour:
//! a stub reports itself, it never panics, and it never writes to disk.

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
fn compiling_a_fixture_reports_a_stub_rather_than_panicking() {
    let nwscript = NwScript::empty(Game::K1);
    let options = CompileOptions::new(&nwscript);

    // TODO(pipeline): once codegen lands, replace this with a byte comparison
    // against a checked in expected output for hello.nss.
    let error = compile_file(fixture("hello.nss"), &options).expect_err("the compiler is a stub");
    assert!(error.is_not_implemented(), "unexpected error: {error}");
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
fn loading_a_routine_table_from_disk_reaches_the_parser() {
    // The stub file is not game data. It only proves the read path works.
    let error =
        NwScript::load(Game::K1, fixture("nwscript_stub.nss")).expect_err("the loader is a stub");
    assert!(
        error.is_not_implemented(),
        "the file should have been read, then handed to the unwritten parser: {error}"
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
    let error = compile("void main() { }", &options).expect_err("the compiler is a stub");
    assert!(error.is_not_implemented());
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
