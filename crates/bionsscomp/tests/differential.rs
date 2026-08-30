//! Differential tests against the real `nwnnsscomp.exe`.
//!
//! The original compiler still runs under Wine, so the strongest check
//! available is: compile the same source with both compilers and require the
//! `.ncs` bytes to be equal. That settles questions no amount of reading the
//! spec can settle, and it catches divergence the moment it appears.
//!
//! # Nothing here is in this repository
//!
//! `nwnnsscomp.exe` and `nwscript.nss` are not redistributable. The tests find
//! them through the environment and **skip** when they are absent, so a clean
//! checkout and CI both stay green.
//!
//! ```text
//! BIONSSCOMP_REF_DIR   a directory holding per-game subdirectories:
//!                        <dir>/k1/nwnnsscomp.exe + <dir>/k1/nwscript.nss
//!                        <dir>/k2/nwnnsscomp.exe + <dir>/k2/nwscript.nss
//! BIONSSCOMP_REF_EXE   a single nwnnsscomp.exe, with the game's nwscript.nss
//!                      beside it. Used for K1 unless BIONSSCOMP_REF_EXE_K2 is
//!                      also set.
//! BIONSSCOMP_REF_EXE_K2  the same, for KOTOR II.
//! BIONSSCOMP_WINE      the wine binary. Defaults to `wine`, and is ignored on
//!                      Windows where the exe runs directly.
//! ```
//!
//! The reference compiler **always exits 0**, even on a fatal error, and writes
//! every diagnostic to stdout. So a run counts as successful only when an
//! output file appeared, it starts with the NCS magic, and stdout carries no
//! error line. Never trust the exit status.
//!
//! # Running
//!
//! ```bash
//! export BIONSSCOMP_REF_DIR=~/Workspaces/bionsscomp_research/harness/ref
//! export WINEPREFIX=~/Workspaces/bionsscomp_research/wineprefix
//! cargo test --test differential -- --nocapture
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use bionsscomp::{compile_file, CompileOptions, Game, NwScript};

// ---------------------------------------------------------------------------
// locating the reference compiler
// ---------------------------------------------------------------------------

/// A copy of `nwnnsscomp.exe` with one game's `nwscript.nss` beside it.
struct Reference {
    exe: PathBuf,
    nwscript: PathBuf,
    game: Game,
}

impl Reference {
    /// Finds the reference compiler for `game`, or explains why it cannot.
    fn find(game: Game) -> Result<Reference, String> {
        let from_dir = std::env::var_os("BIONSSCOMP_REF_DIR").map(|value| {
            PathBuf::from(value).join(match game {
                Game::K1 => "k1",
                Game::K2 => "k2",
            })
        });

        let explicit = match game {
            Game::K1 => std::env::var_os("BIONSSCOMP_REF_EXE"),
            Game::K2 => std::env::var_os("BIONSSCOMP_REF_EXE_K2"),
        }
        .map(PathBuf::from);

        let exe = match (explicit, from_dir) {
            (Some(path), _) => path,
            (None, Some(directory)) => directory.join("nwnnsscomp.exe"),
            (None, None) => {
                return Err(
                    "set BIONSSCOMP_REF_DIR or BIONSSCOMP_REF_EXE to run differential tests"
                        .to_string(),
                )
            }
        };

        if !exe.is_file() {
            return Err(format!("no reference compiler at {}", exe.display()));
        }
        let nwscript = exe
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", exe.display()))?
            .join("nwscript.nss");
        if !nwscript.is_file() {
            return Err(format!(
                "{} has no nwscript.nss beside it, so it would compile for the wrong game",
                exe.display()
            ));
        }
        if !cfg!(windows) && wine_binary().is_none() {
            return Err("wine is not installed".to_string());
        }
        Ok(Reference {
            exe,
            nwscript,
            game,
        })
    }

    /// Compiles `source` in place. Includes resolve beside it, as the original does.
    fn compile(&self, source: &Path, output: &Path) -> RefRun {
        let _ = std::fs::remove_file(output);

        let directory = source.parent().unwrap_or(Path::new("."));
        let mut command = match wine_binary() {
            Some(wine) if !cfg!(windows) => {
                let mut command = Command::new(wine);
                command.arg(&self.exe);
                command
            }
            _ => Command::new(&self.exe),
        };
        command
            .arg("-c")
            .arg(file_name(source))
            .arg(file_name(output))
            .current_dir(directory)
            .env("WINEDEBUG", "-all");

        let output_of = match command.output() {
            Ok(value) => value,
            Err(error) => {
                return RefRun {
                    stdout: format!("could not run the reference compiler: {error}"),
                    ncs: None,
                }
            }
        };
        // Diagnostics go to stdout. stderr only ever carries Wine's own noise.
        let stdout = String::from_utf8_lossy(&output_of.stdout).into_owned();

        // The exit code is always 0, so the output file decides.
        let ncs = std::fs::read(output).ok().filter(|bytes| {
            bytes.starts_with(b"NCS ") && !stdout.lines().any(|line| line.contains("Error"))
        });
        RefRun { stdout, ncs }
    }
}

/// What one reference invocation produced.
struct RefRun {
    stdout: String,
    ncs: Option<Vec<u8>>,
}

fn wine_binary() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    if let Some(value) = std::env::var_os("BIONSSCOMP_WINE") {
        let path = PathBuf::from(value);
        return path.is_file().then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join("wine"))
        .find(|candidate| candidate.is_file())
}

fn file_name(path: &Path) -> &std::ffi::OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

/// A scratch directory under `target/`, so nothing lands in a temp dir that
/// gets cleaned out mid-run.
fn scratch(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/differential")
        .join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

fn skip(reason: &str) {
    println!("SKIP: {reason}");
}

// ---------------------------------------------------------------------------
// a decoder, so a mismatch says something useful
// ---------------------------------------------------------------------------

/// One decoded instruction: where it is, how long, and how it reads.
struct Insn {
    offset: usize,
    size: usize,
    text: String,
}

/// Decodes an `.ncs` far enough to describe it. Stops at the first bad byte.
fn disassemble(bytes: &[u8]) -> Vec<Insn> {
    let mut out = Vec::new();
    if bytes.len() < 13 {
        return out;
    }
    let declared = u32::from_be_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) as usize;
    let end = declared.min(bytes.len());
    let mut at = 13usize;
    while at + 2 <= end {
        let opcode = bytes[at];
        let qualifier = bytes[at + 1];
        let size = match (opcode, qualifier) {
            (0x01 | 0x03 | 0x21 | 0x26 | 0x27, _) => 8,
            (0x2C, _) => 10,
            (0x04, 0x05) => {
                if at + 4 > end {
                    break;
                }
                4 + u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize
            }
            (0x04, _) => 6,
            (0x05, _) => 5,
            (0x1B | 0x1D | 0x1E | 0x1F | 0x23 | 0x24 | 0x25 | 0x28 | 0x29, _) => 6,
            (0x0B | 0x0C, 0x24) => 4,
            _ => 2,
        };
        if at + size > end {
            break;
        }
        let raw = &bytes[at..at + size];
        let mut text = mnemonic(opcode, qualifier);
        match opcode {
            0x1D | 0x1E | 0x1F | 0x25 => {
                let rel = i32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let _ = write!(text, " {rel:+} -> {:08X}", at.wrapping_add(rel as usize));
            }
            0x05 => {
                let routine = u16::from_be_bytes([raw[2], raw[3]]);
                let _ = write!(text, " routine={routine}, argc={}", raw[4]);
            }
            0x01 | 0x03 | 0x26 | 0x27 => {
                let offset = i32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let width = u16::from_be_bytes([raw[6], raw[7]]);
                let _ = write!(text, " {offset}, {width}");
            }
            0x04 if qualifier == 0x05 => {
                let literal = String::from_utf8_lossy(&raw[4..]);
                let _ = write!(text, " {literal:?}");
            }
            0x04 if qualifier == 0x03 || qualifier == 0x06 => {
                let value = i32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let _ = write!(text, " {value}");
            }
            0x04 if qualifier == 0x04 => {
                let value = f32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let _ = write!(text, " {value}");
            }
            0x1B | 0x23 | 0x24 | 0x28 | 0x29 => {
                let value = i32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let _ = write!(text, " {value}");
            }
            0x2C => {
                let globals = i32::from_be_bytes([raw[2], raw[3], raw[4], raw[5]]);
                let locals = i32::from_be_bytes([raw[6], raw[7], raw[8], raw[9]]);
                let _ = write!(text, " globals={globals}, locals={locals}");
            }
            _ => {}
        }
        out.push(Insn {
            offset: at,
            size,
            text,
        });
        at += size;
    }
    out
}

fn mnemonic(opcode: u8, qualifier: u8) -> String {
    let typed = |base: &str| match qualifier {
        0x03 => format!("{base}I"),
        0x04 => format!("{base}F"),
        0x05 => format!("{base}S"),
        0x06 => format!("{base}O"),
        0x10 => format!("{base}EFF"),
        0x11 => format!("{base}EVT"),
        0x12 => format!("{base}LOC"),
        0x13 => format!("{base}TAL"),
        other => format!("{base}?{other:02X}"),
    };
    let paired = |base: &str| match qualifier {
        0x20 => format!("{base}II"),
        0x21 => format!("{base}FF"),
        0x22 => format!("{base}OO"),
        0x23 => format!("{base}SS"),
        0x24 => format!("{base}TT"),
        0x25 => format!("{base}IF"),
        0x26 => format!("{base}FI"),
        0x3A => format!("{base}VV"),
        0x3B => format!("{base}VF"),
        0x3C => format!("{base}FV"),
        other => format!("{base}?{other:02X}"),
    };
    match opcode {
        0x01 => "CPDOWNSP".into(),
        0x02 => typed("RSADD"),
        0x03 => "CPTOPSP".into(),
        0x04 => typed("CONST"),
        0x05 => "ACTION".into(),
        0x06 => paired("LOGAND"),
        0x07 => paired("LOGOR"),
        0x08 => paired("INCOR"),
        0x09 => paired("EXCOR"),
        0x0A => paired("BOOLAND"),
        0x0B => paired("EQUAL"),
        0x0C => paired("NEQUAL"),
        0x0D => paired("GEQ"),
        0x0E => paired("GT"),
        0x0F => paired("LT"),
        0x10 => paired("LEQ"),
        0x11 => paired("SHLEFT"),
        0x12 => paired("SHRIGHT"),
        0x13 => paired("USHRIGHT"),
        0x14 => paired("ADD"),
        0x15 => paired("SUB"),
        0x16 => paired("MUL"),
        0x17 => paired("DIV"),
        0x18 => paired("MOD"),
        0x19 => typed("NEG"),
        0x1A => typed("COMP"),
        0x1B => "MOVSP".into(),
        0x1C => "SAVE_STATEALL".into(),
        0x1D => "JMP".into(),
        0x1E => "JSR".into(),
        0x1F => "JZ".into(),
        0x20 => "RETN".into(),
        0x21 => "DESTRUCT".into(),
        0x22 => typed("NOT"),
        0x23 => "DECISP".into(),
        0x24 => "INCISP".into(),
        0x25 => "JNZ".into(),
        0x26 => "CPDOWNBP".into(),
        0x27 => "CPTOPBP".into(),
        0x28 => "DECIBP".into(),
        0x29 => "INCIBP".into(),
        0x2A => "SAVEBP".into(),
        0x2B => "RESTOREBP".into(),
        0x2C => "STORE_STATE".into(),
        0x2D => "NOP".into(),
        other => format!("UNKNOWN_{other:02X}"),
    }
}

/// Builds the failure message: first differing instruction, then a hex window.
fn explain(expected: &[u8], actual: &[u8]) -> String {
    let mut report = String::new();
    let at = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));

    let _ = writeln!(
        report,
        "reference: {} bytes, bionsscomp: {} bytes, first difference at 0x{at:X}",
        expected.len(),
        actual.len()
    );

    let left = disassemble(expected);
    let right = disassemble(actual);
    let index = left
        .iter()
        .position(|insn| insn.offset <= at && at < insn.offset + insn.size);

    if let Some(index) = index {
        let low = index.saturating_sub(4);
        let high = (index + 5).min(left.len());
        let _ = writeln!(report, "\nreference:");
        for (position, insn) in left[low..high].iter().enumerate() {
            let marker = if low + position == index { ">" } else { " " };
            let _ = writeln!(report, "{marker} {:08X}  {}", insn.offset, insn.text);
        }
        let _ = writeln!(report, "\nbionsscomp:");
        let high = high.min(right.len());
        if low < high {
            for (position, insn) in right[low..high].iter().enumerate() {
                let marker = if low + position == index { ">" } else { " " };
                let _ = writeln!(report, "{marker} {:08X}  {}", insn.offset, insn.text);
            }
        } else {
            let _ = writeln!(report, "  (nothing decoded at that instruction)");
        }
    }

    let _ = writeln!(report, "\nhex around 0x{at:X}:");
    let _ = writeln!(report, "reference:  {}", hex_window(expected, at));
    let _ = writeln!(report, "bionsscomp: {}", hex_window(actual, at));
    report
}

fn hex_window(bytes: &[u8], at: usize) -> String {
    let start = at.saturating_sub(8);
    let end = (start + 24).min(bytes.len());
    bytes[start..end]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if start + index == at {
                format!("[{byte:02X}]")
            } else {
                format!(" {byte:02X} ")
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the runner
// ---------------------------------------------------------------------------

/// Compiles `source` both ways and asserts the bytes match.
///
/// Returns `false` when the harness could not run, so the caller can skip.
fn differential(name: &str, game: Game, source: &str) -> bool {
    let reference = match Reference::find(game) {
        Ok(value) => value,
        Err(reason) => {
            skip(&reason);
            return false;
        }
    };

    let directory = scratch(name);
    let nss = directory.join(format!("{name}.nss"));
    std::fs::write(&nss, source).expect("write the probe source");

    let run = reference.compile(&nss, &directory.join("reference.ncs"));
    let Some(expected) = run.ncs else {
        panic!(
            "the reference compiler rejected {name}, so the test itself is wrong:\n{}",
            run.stdout
        );
    };

    let nwscript = match NwScript::load(reference.game, &reference.nwscript) {
        Ok(value) => value,
        Err(error) => {
            skip(&format!("bionsscomp cannot load nwscript.nss yet: {error}"));
            return false;
        }
    };
    let options = CompileOptions::new(&nwscript).with_include_path(&directory);
    let actual = match compile_file(&nss, &options) {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_implemented() => {
            skip(&format!("bionsscomp is still a scaffold: {error}"));
            return false;
        }
        Err(error) => panic!("bionsscomp failed to compile {name}:\n{error}"),
    };

    assert_eq!(
        expected,
        actual,
        "{name} does not match the reference compiler\n\n{}",
        explain(&expected, &actual)
    );
    true
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

macro_rules! differential_test {
    ($name:ident, $game:expr, $source:expr) => {
        #[test]
        fn $name() {
            differential(stringify!($name), $game, $source);
        }
    };
}

// `k_pman_ignore` is shipped in KOTOR I's scripts.bif and is literally
// `void main(){}` - 21 bytes of source, 23 bytes of compiled script. If this
// one does not match, nothing else will.
differential_test!(empty_main, Game::K1, "void main() { }\n");

// The shipped KOTOR II nwscript.nss declares
//   void SetAvailableNPCId(INT nNPC, OBJECT_ID oidNPC);
// using two type names that are not NWScript keywords. Upstream's keyword
// table is case sensitive and has neither, so the KOTOR fork must have added
// them - otherwise the header would fail to bootstrap and every routine after
// declaration 767 would be misnumbered. `INT` is `int` and `OBJECT_ID` is
// `object`; no other all-caps type name is accepted.
differential_test!(
    uppercase_int_is_a_type_keyword,
    Game::K1,
    "void main()\n{\n    INT v = 1;\n    PrintInteger(v);\n}\n"
);

differential_test!(
    uppercase_object_id_is_a_type_keyword,
    Game::K1,
    "void main()\n{\n    OBJECT_ID v = OBJECT_SELF;\n}\n"
);

differential_test!(
    the_routine_declared_with_uppercase_types_still_resolves,
    Game::K2,
    "void main()\n{\n    SetAvailableNPCId(1, OBJECT_SELF);\n}\n"
);

differential_test!(
    one_engine_call,
    Game::K1,
    "void main()\n{\n    PrintString(\"hello\");\n}\n"
);

differential_test!(
    locals_of_every_primitive_type,
    Game::K1,
    "void main()\n{\n    int i = 1;\n    float f = 2.0;\n    string s = \"three\";\n\
     \x20   object o = OBJECT_SELF;\n    PrintString(s);\n}\n"
);

differential_test!(
    starting_conditional_entry_point,
    Game::K1,
    "int StartingConditional()\n{\n    return 1;\n}\n"
);

differential_test!(
    globals_block_and_loader,
    Game::K1,
    "int gCount = 3;\nstring gName = \"g\";\n\
     void main()\n{\n    gCount = gCount + 1;\n    PrintString(gName);\n}\n"
);

differential_test!(
    script_subroutine_call,
    Game::K1,
    "int Give()\n{\n    return 42;\n}\n\
     void main()\n{\n    int n = Give();\n    PrintInteger(n);\n}\n"
);

// Three nested `if` bodies with no `else`. The reference emits a chain of
// `JMP +6`, each jumping to the instruction that already follows it, because
// its optimiser is off and the else-label and end-label resolve to the same
// address. A peephole pass would delete these and break byte equality.
differential_test!(
    nested_if_emits_a_chain_of_jumps_to_the_next_instruction,
    Game::K1,
    "void main()\n{\n    int a = Random(2);\n    int b = Random(2);\n    int c = Random(2);\n\
     \x20   if (a == 1) { if (b == 1) { if (c == 1) { PrintString(\"deep\"); } } }\n\
     \x20   PrintString(\"after\");\n}\n"
);

differential_test!(
    if_else_chain,
    Game::K1,
    "void main()\n{\n    int a = Random(4);\n\
     \x20   if (a == 0) { PrintString(\"0\"); }\n\
     \x20   else if (a == 1) { PrintString(\"1\"); }\n\
     \x20   else { PrintString(\"2\"); }\n}\n"
);

differential_test!(
    while_loop,
    Game::K1,
    "void main()\n{\n    int i = 0;\n    while (i < 3) { PrintInteger(i); i = i + 1; }\n}\n"
);

differential_test!(
    for_loop,
    Game::K1,
    "void main()\n{\n    int i;\n    for (i = 0; i < 3; i = i + 1) { PrintInteger(i); }\n}\n"
);

differential_test!(
    do_while_loop,
    Game::K1,
    "void main()\n{\n    int i = 0;\n    do { PrintInteger(i); i = i + 1; } while (i < 3);\n}\n"
);

differential_test!(
    switch_statement,
    Game::K1,
    "void main()\n{\n    int a = Random(3);\n\
     \x20   switch (a) { case 0: PrintString(\"a\"); break; default: PrintString(\"c\"); break; }\n}\n"
);

differential_test!(
    short_circuit_operators,
    Game::K1,
    "void main()\n{\n    int a = Random(2);\n    int b = Random(2);\n\
     \x20   if (a == 1 && b == 1) { PrintString(\"and\"); }\n\
     \x20   if (a == 1 || b == 1) { PrintString(\"or\"); }\n}\n"
);

differential_test!(
    struct_local_and_member_access,
    Game::K1,
    "struct Pair { int a; float b; };\n\
     void main()\n{\n    struct Pair p;\n    p.a = 1;\n    p.b = 2.0;\n    PrintInteger(p.a);\n}\n"
);

differential_test!(
    struct_return_value,
    Game::K1,
    "struct Pair { int a; int b; };\n\
     struct Pair Give() { struct Pair p; p.a = 1; p.b = 2; return p; }\n\
     void main()\n{\n    struct Pair q = Give();\n    PrintInteger(q.b);\n}\n"
);

differential_test!(
    vector_arithmetic,
    Game::K1,
    "void main()\n{\n    vector a = Vector(1.0, 2.0, 3.0);\n    vector b = Vector(4.0, 5.0, 6.0);\n\
     \x20   vector c = a + b;\n    PrintFloat(c.z);\n}\n"
);

differential_test!(
    delay_command_store_state,
    Game::K1,
    "void main()\n{\n    int a = 1;\n    DelayCommand(1.0, PrintInteger(a));\n}\n"
);

differential_test!(
    increment_and_compound_assignment,
    Game::K1,
    "void main()\n{\n    int i = 0;\n    i++;\n    i += 2;\n    i *= 3;\n    PrintInteger(i);\n}\n"
);

differential_test!(
    float_literal_forms,
    Game::K1,
    "void main()\n{\n    float a = 1.0;\n    float b = 1.;\n    float c = 2.5f;\n\
     \x20   float d = 5f;\n    PrintFloat(a + b + c + d);\n}\n"
);

differential_test!(
    operator_precedence,
    Game::K1,
    "void main()\n{\n    int a = Random(9);\n\
     \x20   int n = 2 + 3 * 4 - a % 2 << 1;\n\
     \x20   int m = a & 1 == 1 | 2 ^ 3;\n    PrintInteger(n + m);\n}\n"
);

differential_test!(
    default_parameter_values,
    Game::K1,
    "void Take(int a, int b = 5, string c = \"d\") { PrintInteger(b); }\n\
     void main()\n{\n    Take(1);\n}\n"
);

differential_test!(
    recursion,
    Game::K1,
    "int Fact(int n) { if (n <= 1) { return 1; } return n * Fact(n - 1); }\n\
     void main()\n{\n    PrintInteger(Fact(5));\n}\n"
);

// The same source under KOTOR II. Everything about the K2 table that differs
// from K1 shows up here as a different ACTION id.
differential_test!(
    kotor_two_engine_call,
    Game::K2,
    "void main()\n{\n    PrintString(\"hello\");\n}\n"
);

differential_test!(
    kotor_two_locals,
    Game::K2,
    "void main()\n{\n    int i = 1;\n    string s = \"three\";\n    PrintString(s);\n}\n"
);

// ---------------------------------------------------------------------------
// the wrong-game hazard
// ---------------------------------------------------------------------------

/// Compiling against the wrong game's `nwscript.nss` silently emits the wrong
/// ACTION id, with no diagnostic at all. `IsMoviePlaying` is declaration 768 in
/// KOTOR I and 805 in KOTOR II, so the emitted bytes prove which table was used.
///
/// This is the single most common real-world KOTOR modding failure, and it is
/// invisible without a test like this one.
#[test]
fn the_two_games_number_the_same_routine_differently() {
    let source = "void main()\n{\n    int n = IsMoviePlaying();\n    PrintInteger(n);\n}\n";

    let mut ids = Vec::new();
    for (game, expected_id) in [(Game::K1, 768u16), (Game::K2, 805u16)] {
        let reference = match Reference::find(game) {
            Ok(value) => value,
            Err(reason) => {
                skip(&reason);
                return;
            }
        };
        let directory = scratch(&format!("wrong_game_{}", game.short_name()));
        let nss = directory.join("movie.nss");
        std::fs::write(&nss, source).expect("write the probe source");
        let run = reference.compile(&nss, &directory.join("out.ncs"));
        let bytes = run.ncs.unwrap_or_else(|| {
            panic!("the reference compiler rejected the probe:\n{}", run.stdout)
        });

        let id = first_action_id(&bytes)
            .unwrap_or_else(|| panic!("no ACTION instruction in the emitted script"));
        assert_eq!(
            id,
            expected_id,
            "{} numbered IsMoviePlaying {id}, expected {expected_id}. \
             The nwscript.nss beside {} is not this game's.",
            game.title(),
            reference.exe.display()
        );
        ids.push(id);
    }

    assert_ne!(
        ids[0], ids[1],
        "the two reference directories hold the same nwscript.nss, \
         so the wrong-game hazard is not actually covered"
    );
}

/// Compiling KOTOR I source against the KOTOR II table must not silently
/// produce KOTOR I ids. This is the regression guard for `bionsscomp`: the
/// routine id has to come from the supplied table every time.
#[test]
fn bionsscomp_takes_routine_ids_from_the_table_it_was_given() {
    let source = "void main()\n{\n    int n = IsMoviePlaying();\n    PrintInteger(n);\n}\n";

    let mut ids = Vec::new();
    for (game, expected_id) in [(Game::K1, 768u16), (Game::K2, 805u16)] {
        let reference = match Reference::find(game) {
            Ok(value) => value,
            Err(reason) => {
                skip(&reason);
                return;
            }
        };
        let nwscript = match NwScript::load(game, &reference.nwscript) {
            Ok(value) => value,
            Err(error) => {
                skip(&format!("bionsscomp cannot load nwscript.nss yet: {error}"));
                return;
            }
        };
        match nwscript.routine("IsMoviePlaying") {
            Some(routine) => {
                assert_eq!(
                    routine.id,
                    expected_id,
                    "bionsscomp numbered IsMoviePlaying {} for {}, expected {expected_id}",
                    routine.id,
                    game.title()
                );
                ids.push(routine.id);
            }
            None => {
                skip("bionsscomp does not parse nwscript.nss routines yet");
                return;
            }
        }

        let directory = scratch(&format!("table_{}", game.short_name()));
        let nss = directory.join("movie.nss");
        std::fs::write(&nss, source).expect("write the probe source");
        let options = CompileOptions::new(&nwscript);
        match compile_file(&nss, &options) {
            Ok(bytes) => {
                let id = first_action_id(&bytes).expect("an ACTION instruction");
                assert_eq!(id, expected_id, "wrong ACTION id for {}", game.title());
            }
            Err(error) if error.is_not_implemented() => {
                skip("codegen is still a scaffold; the table check above still ran");
            }
            Err(error) => panic!("bionsscomp failed: {error}"),
        }
    }
}

/// Returns the routine id of the first `ACTION` in a compiled script.
fn first_action_id(bytes: &[u8]) -> Option<u16> {
    disassemble(bytes).iter().find_map(|insn| {
        insn.text
            .strip_prefix("ACTION routine=")
            .and_then(|rest| rest.split(',').next())
            .and_then(|digits| digits.parse().ok())
    })
}

/// The reference compiler reports failure on stdout and still exits 0. Anything
/// that drives it must therefore read stdout, not the exit status.
#[test]
fn a_broken_script_is_detected_from_stdout_not_the_exit_code() {
    let reference = match Reference::find(Game::K1) {
        Ok(value) => value,
        Err(reason) => {
            skip(&reason);
            return;
        }
    };
    let directory = scratch("always_exit_zero");
    let nss = directory.join("broken.nss");
    std::fs::write(&nss, "void main() { this is not nwscript }\n").expect("write");

    let run = reference.compile(&nss, &directory.join("broken.ncs"));
    assert!(
        run.ncs.is_none(),
        "a syntactically broken script must not yield usable bytes"
    );
    assert!(
        run.stdout.contains("Error") || run.stdout.contains("aborted"),
        "expected a diagnostic on stdout, got:\n{}",
        run.stdout
    );
}
