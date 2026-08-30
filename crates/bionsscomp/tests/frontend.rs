//! Front end checks against the committed fixtures.
//!
//! These run everywhere, with no game files. They pin the behaviour that
//! surprises people: the string escape rule, the float forms, engine structure
//! types coming from `#define`, and include resolution through a backslash
//! path.

use std::path::PathBuf;

use bionsscomp::ast::{Declaration, EngineType, Expr, Stmt, Type};
use bionsscomp::error::Severity;
use bionsscomp::preprocessor::{self, IncludeResolver};
use bionsscomp::{lexer, parser};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Preprocesses, lexes and parses a fixture, failing on any error.
fn front_end(name: &str) -> (preprocessor::Expansion, parser::Parsed) {
    let path = fixture(name);
    let bytes = std::fs::read(&path).expect("the fixture is committed");
    let source = preprocessor::decode_source(&bytes);

    let mut resolver = IncludeResolver::new();
    resolver.push_path(path.parent().expect("fixtures live in a directory"));

    let expansion =
        preprocessor::expand(&source, &path, &resolver).expect("the fixture preprocesses");
    let lexed = lexer::scan(&expansion.source, &expansion.engine_structures);
    assert!(
        !lexed.has_errors(),
        "lexing {name} failed: {:?}",
        lexed.diagnostics
    );

    let parsed = parser::parse_detailed(lexed.tokens);
    assert!(
        !parsed.has_errors(),
        "parsing {name} failed: {:?}",
        parsed
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| expansion.localize(d.clone()).to_string())
            .collect::<Vec<_>>()
    );

    (expansion, parsed)
}

#[test]
fn the_fixtures_are_where_the_tests_expect_them() {
    assert!(fixture("quirks.nss").is_file());
    assert!(fixture("includes/top.nss").is_file());
    assert!(fixture("includes/incs/inc_b.nss").is_file());
}

#[test]
fn the_quirks_fixture_gets_through_the_front_end() {
    let (expansion, parsed) = front_end("quirks.nss");

    assert_eq!(expansion.engine_structures.lookup("effect"), Some(0));
    assert_eq!(expansion.engine_structures.lookup("talent"), Some(3));
    assert!(!parsed.index.is_include_only());
    assert!(parsed.index.has_main);
}

#[test]
fn shouting_type_names_are_the_ordinary_types() {
    let (_, parsed) = front_end("quirks.nss");
    let global = |name: &str| {
        parsed
            .script
            .declarations
            .iter()
            .find_map(|declaration| match declaration {
                Declaration::Global(global) if global.name == name => Some(global.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name} is declared"))
    };

    assert_eq!(global("gnCounter").ty, Type::Int);
    assert_eq!(global("goTarget").ty, Type::Object);
    assert!(global("MAX_TRIES").is_const);
}

#[test]
fn a_declarator_list_becomes_one_global_per_name() {
    let (_, parsed) = front_end("quirks.nss");
    let floats: Vec<_> = parsed
        .script
        .declarations
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::Global(global) if global.ty == Type::Float => Some(global.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(floats, vec!["gfX", "gfY", "gfZ"]);
}

#[test]
fn struct_fields_keep_their_declaration_order() {
    let (_, parsed) = front_end("quirks.nss");
    let declaration = parsed
        .script
        .struct_decl("Waypoint")
        .expect("Waypoint is declared");
    let names: Vec<_> = declaration
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, vec!["lWhere", "sTag", "sComment"]);
    assert_eq!(
        declaration.fields[0].ty,
        Type::Engine(EngineType::new(2, "location"))
    );
}

#[test]
fn a_prototype_and_its_definition_are_both_kept() {
    let (_, parsed) = front_end("quirks.nss");
    let count = parsed
        .script
        .declarations
        .iter()
        .filter(|declaration| declaration.name() == "ReportProgress")
        .count();
    assert_eq!(count, 2, "the prototype and the definition are both nodes");
    assert_eq!(parsed.script.prototypes().count(), 1);
    assert_eq!(
        parsed.index.function_names,
        vec!["ReportProgress".to_string(), "main".to_string()]
    );
}

#[test]
fn a_tab_escape_is_just_the_letter_t() {
    let (_, parsed) = front_end("quirks.nss");
    let function = parsed
        .script
        .function("ReportProgress")
        .expect("the definition has a body");
    let Stmt::VarDecl(declaration) = &function.body.as_ref().unwrap().statements[0] else {
        panic!("the first statement declares sMessage");
    };
    assert_eq!(declaration.name, "sMessage");

    let Some(Expr::Binary { left, .. }) = declaration.init.as_ref() else {
        panic!("sMessage is built by concatenation");
    };
    let Expr::Binary { left, .. } = left.as_ref() else {
        panic!("the concatenation is left nested");
    };
    assert_eq!(
        **left,
        Expr::StrLit {
            value: b"Step:t".to_vec(),
            span: left.span(),
        },
        "the backslash is dropped and the t stays"
    );
}

#[test]
fn includes_resolve_through_backslash_paths_and_are_not_repeated() {
    let (expansion, parsed) = front_end("includes/top.nss");

    let names: Vec<_> = expansion
        .included_files
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["inc_a.nss".to_string(), "inc_b.nss".to_string()],
        "inc_a is named twice and included once"
    );

    // Both includes contributed their declarations to one translation unit.
    assert_eq!(
        parsed.index.global_names,
        vec!["gnAlphaLoaded".to_string(), "gnBetaLoaded".to_string()]
    );
    assert!(parsed.index.has_main);
}

#[test]
fn diagnostics_name_the_included_file_they_came_from() {
    let path = fixture("includes/top.nss");
    let source = "#include \"inc_a\"\nvoid main() { int 3; }\n";

    let mut resolver = IncludeResolver::new();
    resolver.push_path(path.parent().unwrap());

    let expansion = preprocessor::expand(source, &path, &resolver).expect("the include resolves");
    let lexed = lexer::scan(&expansion.source, &expansion.engine_structures);
    let parsed = parser::parse_detailed(lexed.tokens);

    let reported = expansion.localize(parsed.diagnostics[0].clone());
    assert_eq!(reported.file, "top.nss");
    assert_eq!(
        reported.span.start.line, 2,
        "the include's four lines must not shift the caller's line numbers"
    );
    assert_eq!(
        reported.to_string(),
        "top.nss(2): Error: Syntax error at \"integer constant\""
    );
}

#[test]
fn the_hello_fixture_still_parses() {
    let (_, parsed) = front_end("hello.nss");
    assert!(parsed.index.has_main);
}
